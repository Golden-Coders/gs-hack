//! Pliki `config/` → Postgres → migawka w pamięci (zadania 10.2, 11.1, 11.2, design D5).
//!
//! Co `RELOAD_EVERY`: pliki zmienione od ostatniego razu trafiają do bazy (upsert tylko przy
//! zmianie treści, `version + 1`), usunięte pliki znikają z bazy, błędne pliki trafiają do
//! `p3_load_errors` i NIE nadpisują poprzedniej wersji. Potem migawka jest budowana z bazy
//! (więc widzi też zmiany z API), kroki są klasyfikowane wg rejestru, a `resolved`
//! i `unhandled_actions` zapisywane, gdy się zmieniły.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use common::db::sqlx::{self, PgPool};
use common::kafka::{self, rdkafka::producer::FutureProducer};
use contracts::{topics, ActionDef, PersonDef, RuleDef, RunbookDef, TeamDef, UiEvent, UiEventKind};
use serde_json::{json, Value};

use crate::parse;
use crate::registry::{self, Registry};

/// shortcut: polling zamiast LISTEN/NOTIFY + watchera plików; zmiana widoczna do ~2 s
/// i działa na wolumenach Dockera na macOS. Przejść na NOTIFY, gdyby zapytania co 2 s przeszkadzały.
const RELOAD_EVERY: Duration = Duration::from_secs(2);

/// Akcja → (liczba kroków, które jej używają, runbooki).
pub type Unhandled = BTreeMap<String, (i32, Vec<String>)>;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    pub registry: Registry,
    /// Runbooki po klasyfikacji (`automation`, wymuszone zatwierdzenia).
    pub runbooks: Vec<RunbookDef>,
    pub unhandled: Unhandled,
    /// Tylko odczyt z tabel P2: włączone reguły (przypięcie runbooka do reguły),
    /// osoby i zespoły (akcje `escalate` i `notify`).
    pub rules: Vec<RuleDef>,
    pub people: HashMap<String, PersonDef>,
    pub teams: HashMap<String, TeamDef>,
}

/// Jeden obieg: pliki → baza → migawka. Zwraca `true`, gdy migawka się zmieniła.
pub async fn reload_once(pool: &PgPool, config_dir: &str, snapshot: &ArcSwap<Snapshot>) -> anyhow::Result<bool> {
    sync_files(pool, Path::new(config_dir)).await?;
    let previous = snapshot.load_full();
    let next = load(pool).await?;
    if next.unhandled != previous.unhandled {
        write_unhandled(pool, &next.unhandled).await?;
    }
    if next.runbooks != previous.runbooks {
        // Router (P2) przeładowuje runbooki MANUAL tylko po tym powiadomieniu (design D5).
        sqlx::query("SELECT pg_notify('config_changed', 'runbooks')").execute(pool).await?;
    }
    if next.runbooks.len() != previous.runbooks.len() || next.registry.len() != previous.registry.len() {
        tracing::info!(runbooks = next.runbooks.len(), actions = next.registry.len(), unhandled = next.unhandled.len(), "p3 config reloaded");
    }
    let changed = *previous != next;
    snapshot.store(Arc::new(next));
    Ok(changed)
}

/// Pętla przeładowania. Błąd jednego obiegu (np. chwilowy brak bazy) tylko loguje.
/// Zmiana = `ui.events {config_changed}`, żeby ekrany akcji i runbooków się odświeżyły.
pub async fn run(pool: PgPool, config_dir: String, snapshot: Arc<ArcSwap<Snapshot>>, producer: FutureProducer) {
    loop {
        tokio::time::sleep(RELOAD_EVERY).await;
        match reload_once(&pool, &config_dir, &snapshot).await {
            Ok(true) => {
                let event = UiEvent { alert_id: None, kind: UiEventKind::ConfigChanged, payload: json!({ "kind": "runbooks" }), at: chrono::Utc::now() };
                if let Err(e) = kafka::publish(&producer, topics::UI_EVENTS, "config_changed", &event).await {
                    tracing::warn!(error = %e, "publishing config_changed failed");
                }
            }
            Ok(false) => {}
            Err(e) => tracing::warn!(error = %e, "p3 config reload failed"),
        }
    }
}

pub async fn sync_files(pool: &PgPool, config_dir: &Path) -> anyhow::Result<()> {
    // Akcje wbudowane też trafiają do `actions` (source_file = 'builtin'), żeby api widziało cały rejestr.
    for action in Registry::builtins() {
        let def = serde_json::to_value(&action)?;
        sqlx::query(
            "INSERT INTO actions (name, definition, source_file, source_text) VALUES ($1, $2, 'builtin', $3)
             ON CONFLICT (name) DO UPDATE SET definition = EXCLUDED.definition, source_file = 'builtin',
                source_text = EXCLUDED.source_text, version = actions.version + 1, updated_at = now()
             WHERE actions.source_text IS DISTINCT FROM EXCLUDED.source_text",
        )
        .bind(&action.name)
        .bind(&def)
        .bind(def.to_string())
        .execute(pool)
        .await?;
    }
    sync_dir(pool, &config_dir.join("actions"), "actions", "name", |file, text| {
        let def = parse::parse_action(file, text)?;
        registry::validate_action(&def).map_err(|e| format!("{file}: {e}"))?;
        Ok((def.name.clone(), serde_json::to_value(&def).expect("serializable")))
    })
    .await?;
    sync_dir(pool, &config_dir.join("runbooks"), "runbooks", "id", |file, text| {
        let def = parse::parse_runbook(file, text)?;
        Ok((def.id.clone(), serde_json::to_value(&def).expect("serializable")))
    })
    .await
}

/// `table`/`key_col` to stałe z kodu (nie dane od użytkownika), więc `format!` jest bezpieczny.
async fn sync_dir(
    pool: &PgPool,
    dir: &Path,
    table: &str,
    key_col: &str,
    parse: impl Fn(&str, &str) -> Result<(String, Value), String>,
) -> anyhow::Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let mut paths: Vec<_> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && !p.file_name().and_then(|n| n.to_str()).unwrap_or(".").starts_with('.'))
        .collect();
    paths.sort();

    let prefix = format!("{}/", dir.display());
    let mut present = Vec::new();
    let mut failed = Vec::new();
    let mut seen_keys: HashMap<String, String> = HashMap::new();
    for path in paths {
        let file = path.display().to_string();
        present.push(file.clone());
        let parsed = std::fs::read_to_string(&path).map_err(|e| format!("{file}: {e}")).and_then(|text| {
            let (key, def) = parse(&file, &text)?;
            if let Some(other) = seen_keys.insert(key.clone(), file.clone()) {
                return Err(format!("{file}: `{key}` jest już zdefiniowane w {other}"));
            }
            Ok((key, def, text))
        });
        match parsed {
            Ok((key, def, text)) => {
                let changed = sqlx::query(&format!(
                    "INSERT INTO {table} ({key_col}, definition, source_file, source_text) VALUES ($1, $2, $3, $4)
                     ON CONFLICT ({key_col}) DO UPDATE SET definition = EXCLUDED.definition,
                        source_file = EXCLUDED.source_file, source_text = EXCLUDED.source_text,
                        version = {table}.version + 1, updated_at = now()
                     WHERE {table}.source_text IS DISTINCT FROM EXCLUDED.source_text"
                ))
                .bind(&key)
                .bind(&def)
                .bind(&file)
                .bind(&text)
                .execute(pool)
                .await?
                .rows_affected();
                if changed > 0 {
                    tracing::info!(file, key, "{table}: loaded from file");
                }
                // Zmieniono id w pliku: stary wpis z tego pliku znika.
                sqlx::query(&format!("DELETE FROM {table} WHERE source_file = $1 AND {key_col} <> $2"))
                    .bind(&file)
                    .bind(&key)
                    .execute(pool)
                    .await?;
            }
            Err(error) => {
                failed.push(file.clone());
                let changed = sqlx::query(
                    "INSERT INTO runbook_load_errors (source_file, error) VALUES ($1, $2)
                     ON CONFLICT (source_file) DO UPDATE SET error = EXCLUDED.error, updated_at = now()
                     WHERE runbook_load_errors.error IS DISTINCT FROM EXCLUDED.error",
                )
                .bind(&file)
                .bind(&error)
                .execute(pool)
                .await?
                .rows_affected();
                if changed > 0 {
                    tracing::warn!(error, "{table}: invalid file, previous version stays active");
                }
            }
        }
    }

    // Plik usunięty => wpis znika. Plik błędny zostaje w `present`, więc jego stara wersja działa dalej.
    sqlx::query(&format!("DELETE FROM {table} WHERE left(source_file, length($1)) = $1 AND NOT (source_file = ANY($2))"))
        .bind(&prefix)
        .bind(&present)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM runbook_load_errors WHERE left(source_file, length($1)) = $1 AND NOT (source_file = ANY($2))")
        .bind(&prefix)
        .bind(&failed)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn load(pool: &PgPool) -> anyhow::Result<Snapshot> {
    let rows: Vec<(String, Value, i64)> = sqlx::query_as("SELECT name, definition, version FROM actions WHERE source_file IS DISTINCT FROM 'builtin' ORDER BY name").fetch_all(pool).await?;
    let mut custom = Vec::new();
    for (name, definition, version) in rows {
        let parsed = serde_json::from_value::<ActionDef>(definition).map_err(|e| e.to_string()).and_then(|a| {
            registry::validate_action(&a)?;
            Ok(a)
        });
        match parsed {
            Ok(action) => custom.push(ActionDef { version, ..action }),
            Err(error) => tracing::warn!(name, error, "skipping invalid action from database"),
        }
    }
    let registry = Registry::new(custom);

    let rows: Vec<(String, Value, Option<Value>, i64)> =
        sqlx::query_as("SELECT id, definition, resolved, version FROM runbooks ORDER BY id").fetch_all(pool).await?;
    let mut runbooks = Vec::new();
    let mut unhandled = Unhandled::new();
    for (id, definition, stored_resolved, version) in rows {
        let def = match serde_json::from_value::<RunbookDef>(definition) {
            Ok(def) => RunbookDef { version, ..def },
            Err(error) => {
                tracing::warn!(id, %error, "skipping invalid runbook from database");
                continue;
            }
        };
        let resolved = registry.classify(&def);
        for action in registry::unhandled_actions(&resolved) {
            let entry = unhandled.entry(action.to_string()).or_default();
            entry.0 += 1;
            if !entry.1.contains(&id) {
                entry.1.push(id.clone());
            }
        }
        let value = serde_json::to_value(&resolved)?;
        if stored_resolved.as_ref() != Some(&value) {
            sqlx::query("UPDATE runbooks SET resolved = $2 WHERE id = $1").bind(&id).bind(&value).execute(pool).await?;
        }
        runbooks.push(resolved);
    }
    Ok(Snapshot {
        registry,
        runbooks,
        unhandled,
        rules: read_defs(pool, "SELECT def FROM rules WHERE enabled ORDER BY id").await?,
        people: read_defs::<PersonDef>(pool, "SELECT def FROM people ORDER BY id").await?.into_iter().map(|p| (p.id.clone(), p)).collect(),
        teams: read_defs::<TeamDef>(pool, "SELECT def FROM teams ORDER BY id").await?.into_iter().map(|t| (t.id.clone(), t)).collect(),
    })
}

/// Definicje JSONB z tabel innego strumienia. Wiersz, który nie pasuje do kontraktu, pomijamy
/// (jego właściciel waliduje zapis; tu nie spamujemy logu co 2 s).
async fn read_defs<T: serde::de::DeserializeOwned>(pool: &PgPool, sql: &str) -> anyhow::Result<Vec<T>> {
    let rows: Vec<Value> = sqlx::query_scalar(sql).fetch_all(pool).await?;
    Ok(rows.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect())
}

async fn write_unhandled(pool: &PgPool, unhandled: &Unhandled) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM unhandled_actions").execute(&mut *tx).await?;
    for (action, (occurrences, runbook_ids)) in unhandled {
        sqlx::query("INSERT INTO unhandled_actions (action, occurrences, runbook_ids) VALUES ($1, $2, $3)")
            .bind(action)
            .bind(occurrences)
            .bind(runbook_ids)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}
