//! Konfiguracja routera (P2): reguły (`config/p2/rules/*.yaml`), osoby i zespoły (`config/p2/people.yaml`),
//! oraz ładowanie runbooków MANUAL z bazy (tabela `runbooks` P3, tylko do odczytu) z hot reloadem
//! przez `PgListener` na kanale `config_changed` i `ArcSwap` (design D5, D10; tasks 7.1, 8.1, 8.3).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};

use anyhow::Context;
use common::db::sqlx::{postgres::PgListener, PgPool, Row};
use common::kafka::rdkafka::producer::FutureProducer;
use contracts::{
    topics, Channel, Condition, PersonDef, PreferenceDef, RuleDef, RunbookDef, RunbookKind,
    Severity, TeamDef, UiEvent, UiEventKind,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::Shutdown;

/// Skompilowana reguła gotowa do szybkiej ewaluacji w pamięci.
#[derive(Debug, Clone)]
pub struct CompiledRule {
    pub def: RuleDef,
    pub compiled: condition::Compiled,
}

impl CompiledRule {
    pub fn compile(def: RuleDef) -> Result<Self, condition::Error> {
        if let Some(title) = &def.presentation.title {
            condition::validate_template(title)?;
        }
        if let Some(body) = &def.presentation.body {
            condition::validate_template(body)?;
        }
        let compiled = condition::compile(&def.condition)?;
        Ok(Self { def, compiled })
    }
}

/// Skompilowany runbook typu MANUAL (dołączany do powiadomień przez router wg design D10).
#[derive(Debug, Clone)]
pub struct CompiledManualRunbook {
    pub def: RunbookDef,
    pub compiled: Option<condition::Compiled>,
}

impl CompiledManualRunbook {
    pub fn compile(def: RunbookDef) -> Result<Self, condition::Error> {
        let compiled = match &def.matches {
            Some(cond) => Some(condition::compile(cond)?),
            None => None,
        };
        Ok(Self { def, compiled })
    }
}

/// Atomowy snapshot konfiguracji routera w pamięci.
#[derive(Debug, Clone, Default)]
pub struct RouterSnapshot {
    pub rules: Vec<CompiledRule>,
    pub people: HashMap<String, PersonDef>,
    pub teams: HashMap<String, TeamDef>,
    pub manual_runbooks: Vec<CompiledManualRunbook>,
}

/// Współdzielony, atomowo podmieniany magazyn konfiguracji routera.
#[derive(Debug, Clone, Default)]
pub struct RouterStore {
    inner: Arc<RwLock<Arc<RouterSnapshot>>>,
}

impl RouterStore {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(Arc::new(RouterSnapshot::default()))),
        }
    }

    pub fn load_full(&self) -> Arc<RouterSnapshot> {
        self.inner
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn load(&self) -> Arc<RouterSnapshot> {
        self.load_full()
    }

    pub fn store(&self, snap: Arc<RouterSnapshot>) {
        *self.inner.write().unwrap_or_else(|e| e.into_inner()) = snap;
    }
}

pub fn new_store() -> RouterStore {
    RouterStore::new()
}

/// Struktura pliku `config/p2/people.yaml`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PeopleConfigFile {
    #[serde(default)]
    pub people: Vec<PersonDef>,
    #[serde(default)]
    pub teams: Vec<TeamDef>,
}

fn rule_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let mut files: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            matches!(
                p.extension().and_then(|e| e.to_str()),
                Some("yaml" | "yml")
            )
        })
        .collect();
    files.sort();
    files
}

/// Parsuje i waliduje definicję reguły z YAML (warunek + szablony prezentacji).
pub fn parse_and_validate_rule_yaml(yaml_str: &str) -> anyhow::Result<RuleDef> {
    let def: RuleDef = serde_yaml::from_str(yaml_str).context("parsing rule YAML")?;
    let _ = CompiledRule::compile(def.clone()).context("validating rule condition/templates")?;
    Ok(def)
}

fn read_rule_def(path: &Path) -> anyhow::Result<RuleDef> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("reading rule file {}", path.display()))?;
    parse_and_validate_rule_yaml(&content)
}

/// Seed: wstawia reguły z `config/p2/rules/*.yaml` do tabeli `rules`, nie nadpisując zmian z API.
pub async fn seed_rules(pool: &PgPool, rules_dir: &Path) -> anyhow::Result<()> {
    for path in rule_files(rules_dir) {
        match read_rule_def(&path) {
            Ok(mut def) => {
                if def.version <= 0 {
                    def.version = 1;
                }
                let json = serde_json::to_value(&def)?;
                common::db::sqlx::query(
                    "INSERT INTO rules (id, name, team, def, enabled, version, updated_at)
                     VALUES ($1, $2, $3, $4, $5, $6, now())
                     ON CONFLICT (id) DO NOTHING",
                )
                .bind(&def.id)
                .bind(&def.name)
                .bind(&def.team)
                .bind(json)
                .bind(def.enabled)
                .bind(def.version)
                .execute(pool)
                .await?;
            }
            Err(e) => {
                tracing::warn!(file = %path.display(), error = %e, "invalid rule file skipped during seed")
            }
        }
    }
    Ok(())
}

fn severity_to_str(s: Severity) -> &'static str {
    match s {
        Severity::Info => "info",
        Severity::Warning => "warning",
        Severity::Error => "error",
        Severity::Critical => "critical",
    }
}

/// Parsuje plik `people.yaml` i synchronizuje tabele `people`, `teams`, `team_members` oraz `preferences`.
/// Zwraca `true`, jeśli zawartość uległa zmianie (i wysyła `NOTIFY config_changed, 'people'`).
pub async fn sync_people_yaml(pool: &PgPool, file_path: &Path) -> anyhow::Result<bool> {
    if !file_path.exists() {
        return Ok(false);
    }
    let content = std::fs::read_to_string(file_path)
        .with_context(|| format!("reading people file {}", file_path.display()))?;
    let parsed: PeopleConfigFile =
        serde_yaml::from_str(&content).context("parsing people.yaml")?;
    sync_people_config(pool, &parsed).await
}

/// Synchronizuje strukturę `PeopleConfigFile` do Postgresa w jednej transakcji.
pub async fn sync_people_config(pool: &PgPool, cfg: &PeopleConfigFile) -> anyhow::Result<bool> {
    let mut tx = pool.begin().await?;
    let mut changed = false;

    let person_ids: Vec<String> = cfg.people.iter().map(|p| p.id.clone()).collect();
    if person_ids.is_empty() {
        let r = common::db::sqlx::query("DELETE FROM people")
            .execute(&mut *tx)
            .await?;
        if r.rows_affected() > 0 {
            changed = true;
        }
    } else {
        let r = common::db::sqlx::query("DELETE FROM people WHERE NOT (id = ANY($1))")
            .bind(&person_ids)
            .execute(&mut *tx)
            .await?;
        if r.rows_affected() > 0 {
            changed = true;
        }
    }

    for person in &cfg.people {
        let contacts_json = serde_json::to_value(&person.contacts)?;
        let fallback_json = serde_json::to_value(&person.fallback)?;
        let def_json = serde_json::to_value(person)?;
        let r = common::db::sqlx::query(
            "INSERT INTO people (id, name, timezone, contacts, fallback, def, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, now())
             ON CONFLICT (id) DO UPDATE
             SET name = EXCLUDED.name,
                 timezone = EXCLUDED.timezone,
                 contacts = EXCLUDED.contacts,
                 fallback = EXCLUDED.fallback,
                 def = EXCLUDED.def,
                 updated_at = now()
             WHERE people.def IS DISTINCT FROM EXCLUDED.def",
        )
        .bind(&person.id)
        .bind(&person.name)
        .bind(&person.timezone)
        .bind(contacts_json)
        .bind(fallback_json)
        .bind(def_json)
        .execute(&mut *tx)
        .await?;
        if r.rows_affected() > 0 {
            changed = true;
        }
    }

    let team_ids: Vec<String> = cfg.teams.iter().map(|t| t.id.clone()).collect();
    if team_ids.is_empty() {
        let r = common::db::sqlx::query("DELETE FROM teams")
            .execute(&mut *tx)
            .await?;
        if r.rows_affected() > 0 {
            changed = true;
        }
    } else {
        let r = common::db::sqlx::query("DELETE FROM teams WHERE NOT (id = ANY($1))")
            .bind(&team_ids)
            .execute(&mut *tx)
            .await?;
        if r.rows_affected() > 0 {
            changed = true;
        }
    }

    for team in &cfg.teams {
        let def_json = serde_json::to_value(team)?;
        let r = common::db::sqlx::query(
            "INSERT INTO teams (id, name, def, updated_at)
             VALUES ($1, $2, $3, now())
             ON CONFLICT (id) DO UPDATE
             SET name = EXCLUDED.name,
                 def = EXCLUDED.def,
                 updated_at = now()
             WHERE teams.def IS DISTINCT FROM EXCLUDED.def",
        )
        .bind(&team.id)
        .bind(&team.name)
        .bind(def_json)
        .execute(&mut *tx)
        .await?;
        if r.rows_affected() > 0 {
            changed = true;
        }
    }

    // Zawsze utrzymujemy spójność tabel pomocniczych team_members i preferences,
    // gdy zaszła zmiana lub tabele są puste.
    let tm_count: i64 = common::db::sqlx::query("SELECT count(*) AS n FROM team_members")
        .fetch_one(&mut *tx)
        .await?
        .get("n");
    let expected_tm_count: usize = cfg.teams.iter().map(|t| t.members.len()).sum();

    if changed || (tm_count as usize) != expected_tm_count {
        common::db::sqlx::query("DELETE FROM team_members")
            .execute(&mut *tx)
            .await?;
        for team in &cfg.teams {
            for member in &team.members {
                common::db::sqlx::query(
                    "INSERT INTO team_members (team_id, person_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
                )
                .bind(&team.id)
                .bind(member)
                .execute(&mut *tx)
                .await?;
            }
        }

        common::db::sqlx::query("DELETE FROM preferences")
            .execute(&mut *tx)
            .await?;
        for person in &cfg.people {
            insert_preferences(&mut tx, "person", &person.id, &person.preferences).await?;
        }
        for team in &cfg.teams {
            insert_preferences(&mut tx, "team", &team.id, &team.preferences).await?;
        }
    }

    tx.commit().await?;

    if changed {
        tracing::info!(
            people = cfg.people.len(),
            teams = cfg.teams.len(),
            "people.yaml synced to postgres"
        );
        let _ = common::db::sqlx::query("SELECT pg_notify('config_changed', 'people')")
            .execute(pool)
            .await;
    }

    Ok(changed)
}

async fn insert_preferences(
    tx: &mut common::db::sqlx::Transaction<'_, common::db::sqlx::Postgres>,
    owner_type: &str,
    owner_id: &str,
    prefs: &[PreferenceDef],
) -> anyhow::Result<()> {
    for pref in prefs {
        let channels_json = serde_json::to_value(&pref.channels)?;
        common::db::sqlx::query(
            "INSERT INTO preferences (owner_type, owner_id, severity, channels, hours)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(owner_type)
        .bind(owner_id)
        .bind(severity_to_str(pref.severity))
        .bind(channels_json)
        .bind(&pref.hours)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// Wczytuje pełny `RouterSnapshot` z Postgresa (reguły, osoby, zespoły, runbooki MANUAL) i podmienia w `store`.
pub async fn load_snapshot(pool: &PgPool, store: &RouterStore) -> anyhow::Result<()> {
    let rules = load_rules_from_db(pool).await?;
    let people = load_people_from_db(pool).await?;
    let teams = load_teams_from_db(pool).await?;
    let manual_runbooks = load_manual_runbooks_from_db(pool).await;

    tracing::info!(
        rules = rules.len(),
        people = people.len(),
        teams = teams.len(),
        manual_runbooks = manual_runbooks.len(),
        "router configuration snapshot loaded"
    );

    store.store(Arc::new(RouterSnapshot {
        rules,
        people,
        teams,
        manual_runbooks,
    }));
    Ok(())
}

async fn load_rules_from_db(pool: &PgPool) -> anyhow::Result<Vec<CompiledRule>> {
    let rows = common::db::sqlx::query(
        "SELECT id, def, enabled, version FROM rules ORDER BY id",
    )
    .fetch_all(pool)
    .await?;

    let mut compiled_rules = Vec::with_capacity(rows.len());
    for row in rows {
        let id: String = row.get("id");
        let def_val: Value = row.get("def");
        let enabled: bool = row.get("enabled");
        let version: i64 = row.get("version");

        let parsed = serde_json::from_value::<RuleDef>(def_val)
            .map_err(anyhow::Error::from)
            .and_then(|mut d| {
                d.enabled = enabled;
                d.version = version;
                CompiledRule::compile(d).map_err(anyhow::Error::from)
            });

        match parsed {
            Ok(cr) => compiled_rules.push(cr),
            Err(e) => tracing::warn!(rule_id = %id, error = %e, "rule does not compile, skipped"),
        }
    }
    Ok(compiled_rules)
}

async fn load_people_from_db(pool: &PgPool) -> anyhow::Result<HashMap<String, PersonDef>> {
    let rows = common::db::sqlx::query("SELECT id, def FROM people")
        .fetch_all(pool)
        .await?;
    let mut map = HashMap::with_capacity(rows.len());
    for row in rows {
        let id: String = row.get("id");
        let def_val: Value = row.get("def");
        match serde_json::from_value::<PersonDef>(def_val) {
            Ok(p) => {
                map.insert(id, p);
            }
            Err(e) => tracing::warn!(person_id = %id, error = %e, "invalid person def in db"),
        }
    }
    Ok(map)
}

async fn load_teams_from_db(pool: &PgPool) -> anyhow::Result<HashMap<String, TeamDef>> {
    let rows = common::db::sqlx::query("SELECT id, def FROM teams")
        .fetch_all(pool)
        .await?;
    let mut map = HashMap::with_capacity(rows.len());
    for row in rows {
        let id: String = row.get("id");
        let def_val: Value = row.get("def");
        match serde_json::from_value::<TeamDef>(def_val) {
            Ok(t) => {
                map.insert(id, t);
            }
            Err(e) => tracing::warn!(team_id = %id, error = %e, "invalid team def in db"),
        }
    }
    Ok(map)
}

/// Bezpiecznie wczytuje runbooki MANUAL z tabeli `runbooks` (właściciel: P3, tylko do odczytu wg D3/D10).
/// Jeśli tabela `runbooks` jeszcze nie istnieje (migracja P3 jeszcze nie dodana), zwraca pustą listę.
async fn load_manual_runbooks_from_db(pool: &PgPool) -> Vec<CompiledManualRunbook> {
    let exists: Option<String> = match common::db::sqlx::query_scalar(
        "SELECT to_regclass('public.runbooks')::text",
    )
    .fetch_one(pool)
    .await
    {
        Ok(v) => v,
        Err(_) => return vec![],
    };

    if exists.is_none() {
        return vec![];
    }

    let rows = match common::db::sqlx::query("SELECT * FROM runbooks")
        .fetch_all(pool)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(error = %e, "could not query runbooks table");
            return vec![];
        }
    };

    let mut out = Vec::new();
    for row in rows {
        let rb_def: Option<RunbookDef> = if let Ok(def_val) = row.try_get::<Value, _>("def") {
            serde_json::from_value::<RunbookDef>(def_val).ok()
        } else {
            let id = row.try_get::<String, _>("id").ok();
            let title = row
                .try_get::<String, _>("title")
                .or_else(|_| row.try_get::<String, _>("name"))
                .ok();
            let kind_str = row.try_get::<String, _>("kind").unwrap_or_default();
            if !kind_str.eq_ignore_ascii_case("manual") {
                continue;
            }
            let matches_cond: Option<Condition> = row
                .try_get::<Option<Value>, _>("matches")
                .ok()
                .flatten()
                .and_then(|v| serde_json::from_value(v).ok());
            let rule_ids: Vec<String> = row
                .try_get::<Value, _>("rule_ids")
                .ok()
                .and_then(|v| serde_json::from_value(v).ok())
                .or_else(|| row.try_get::<Vec<String>, _>("rule_ids").ok())
                .unwrap_or_default();
            let body: Option<String> = row.try_get::<Option<String>, _>("body").ok().flatten();
            let source_file: Option<String> =
                row.try_get::<Option<String>, _>("source_file").ok().flatten();
            let version: i64 = row.try_get::<i64, _>("version").unwrap_or(1);

            match (id, title) {
                (Some(id), Some(title)) => Some(RunbookDef {
                    id,
                    title,
                    kind: RunbookKind::Manual,
                    matches: matches_cond,
                    rule_ids,
                    body,
                    steps: vec![],
                    source_file,
                    version,
                }),
                _ => None,
            }
        };

        if let Some(def) = rb_def {
            if def.kind == RunbookKind::Manual {
                match CompiledManualRunbook::compile(def) {
                    Ok(compiled) => out.push(compiled),
                    Err(e) => tracing::warn!(error = %e, "manual runbook condition failed to compile"),
                }
            }
        }
    }
    out
}

/// Sprawdza zmiany plików w `config/p2/rules/` oraz `config/p2/people.yaml`.
pub async fn scan_p2_files(
    pool: &PgPool,
    rules_dir: &Path,
    people_file: &Path,
    seen_rules: &mut HashMap<PathBuf, (SystemTime, String)>,
    seen_people: &mut Option<SystemTime>,
) {
    // 1. Scan rules directory
    let current_files = rule_files(rules_dir);
    let current_set: HashSet<PathBuf> = current_files.iter().cloned().collect();

    // Handle deleted rule files
    let deleted_paths: Vec<PathBuf> = seen_rules
        .keys()
        .filter(|p| !current_set.contains(*p))
        .cloned()
        .collect();
    let mut rules_notified = false;
    for del_path in deleted_paths {
        if let Some((_, rule_id)) = seen_rules.remove(&del_path) {
            if let Ok(r) = common::db::sqlx::query("DELETE FROM rules WHERE id = $1")
                .bind(&rule_id)
                .execute(pool)
                .await
            {
                if r.rows_affected() > 0 {
                    tracing::info!(rule_id = %rule_id, file = %del_path.display(), "rule file deleted, removed from db");
                    rules_notified = true;
                }
            }
        }
    }

    for path in current_files {
        let Ok(mtime) = std::fs::metadata(&path).and_then(|m| m.modified()) else {
            continue;
        };
        if seen_rules.get(&path).map(|(t, _)| *t) == Some(mtime) {
            continue;
        }
        let mut def = match read_rule_def(&path) {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(file = %path.display(), error = %e, "invalid rule file, skipping hot reload");
                seen_rules.insert(
                    path.clone(),
                    (
                        mtime,
                        seen_rules
                            .get(&path)
                            .map(|(_, id)| id.clone())
                            .unwrap_or_default(),
                    ),
                );
                continue;
            }
        };
        if def.version <= 0 {
            def.version = 1;
        }
        seen_rules.insert(path.clone(), (mtime, def.id.clone()));

        let Ok(json) = serde_json::to_value(&def) else {
            continue;
        };
        let res = common::db::sqlx::query(
            "INSERT INTO rules (id, name, team, def, enabled, version, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, now())
             ON CONFLICT (id) DO UPDATE
             SET name = EXCLUDED.name,
                 team = EXCLUDED.team,
                 def = EXCLUDED.def,
                 enabled = EXCLUDED.enabled,
                 version = rules.version + 1,
                 updated_at = now()
             WHERE rules.def IS DISTINCT FROM EXCLUDED.def
                OR rules.enabled IS DISTINCT FROM EXCLUDED.enabled
                OR rules.name IS DISTINCT FROM EXCLUDED.name
                OR rules.team IS DISTINCT FROM EXCLUDED.team",
        )
        .bind(&def.id)
        .bind(&def.name)
        .bind(&def.team)
        .bind(json)
        .bind(def.enabled)
        .bind(def.version)
        .execute(pool)
        .await;

        match res {
            Ok(r) if r.rows_affected() > 0 => {
                tracing::info!(rule_id = %def.id, "rule file changed, upserted");
                rules_notified = true;
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(rule_id = %def.id, error = %e, "rule upsert failed"),
        }
    }

    if rules_notified {
        let _ = common::db::sqlx::query("SELECT pg_notify('config_changed', 'rules')")
            .execute(pool)
            .await;
    }

    // 2. Scan people.yaml
    if let Ok(mtime) = std::fs::metadata(people_file).and_then(|m| m.modified()) {
        if *seen_people != Some(mtime) {
            *seen_people = Some(mtime);
            if let Err(e) = sync_people_yaml(pool, people_file).await {
                tracing::warn!(file = %people_file.display(), error = %e, "people.yaml hot reload failed");
            }
        }
    }
}

/// Seed + wstępne ładowanie + polling plików + nasłuch `config_changed` (reguły, osoby/zespoły, runbooki).
pub async fn run_config(
    database_url: String,
    config_dir: String,
    pool: PgPool,
    producer: FutureProducer,
    store: RouterStore,
    mut shutdown: Shutdown,
) -> anyhow::Result<()> {
    let p2_dir = Path::new(&config_dir).join("p2");
    let rules_dir = p2_dir.join("rules");
    let people_file = p2_dir.join("people.yaml");

    seed_rules(&pool, &rules_dir).await?;
    if let Err(e) = sync_people_yaml(&pool, &people_file).await {
        tracing::warn!(error = %e, "initial people.yaml sync failed");
    }

    let mut seen_rules: HashMap<PathBuf, (SystemTime, String)> = HashMap::new();
    for p in rule_files(&rules_dir) {
        if let Ok(m) = std::fs::metadata(&p).and_then(|m| m.modified()) {
            let rule_id = read_rule_def(&p).map(|d| d.id).unwrap_or_default();
            seen_rules.insert(p, (m, rule_id));
        }
    }
    let mut seen_people: Option<SystemTime> = std::fs::metadata(&people_file)
        .and_then(|m| m.modified())
        .ok();

    let mut listener = PgListener::connect(&database_url).await?;
    listener.listen("config_changed").await?;
    load_snapshot(&pool, &store).await?;

    // Polling co 2 s (zgodnie z `crates/lifecycle/src/config.rs`, bind-mount Dockera na macOS).
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    loop {
        tokio::select! {
            _ = shutdown.changed() => return Ok(()),
            _ = tick.tick() => {
                scan_p2_files(&pool, &rules_dir, &people_file, &mut seen_rules, &mut seen_people).await;
            }
            n = listener.recv() => match n {
                Ok(n) if matches!(n.payload(), "rules" | "people" | "runbooks") => {
                    let kind = n.payload().to_string();
                    if let Err(e) = load_snapshot(&pool, &store).await {
                        tracing::warn!(error = %e, kind = %kind, "router snapshot reload failed");
                        continue;
                    }
                    let ev = UiEvent {
                        alert_id: None,
                        kind: UiEventKind::ConfigChanged,
                        payload: json!({ "kind": kind }),
                        at: chrono::Utc::now(),
                    };
                    let _ = common::kafka::publish(&producer, topics::UI_EVENTS, "config_changed", &ev).await;
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(error = %e, "router config listener error");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    let _ = load_snapshot(&pool, &store).await;
                }
            }
        }
    }
}

/// Buduje łańcuch fallbacku: najpierw kanał podstawowy, potem zapasowe z profilu osoby
/// (bez duplikatów i bez `Ui`), a `Channel::Ui` jest ZAWSZE ostatnim kanałem zapasowym
/// (spec `recipient-routing`: "UI SHALL być zawsze ostatnim kanałem zapasowym").
pub fn default_fallback_chain(primary: Channel, person_fallbacks: &[Channel]) -> Vec<Channel> {
    if primary == Channel::Ui {
        return vec![Channel::Ui];
    }
    let mut chain = vec![primary];
    for fb in person_fallbacks {
        if *fb != Channel::Ui && !chain.contains(fb) {
            chain.push(*fb);
        }
    }
    chain.push(Channel::Ui);
    chain
}
