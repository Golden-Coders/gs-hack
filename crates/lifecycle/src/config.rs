//! Konfiguracja mappingów: seed z plików, hot reload (polling), ładowanie z Postgresa (design D5).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use arc_swap::ArcSwap;
use common::db::sqlx::{postgres::PgListener, PgPool, Row};
use common::kafka::rdkafka::producer::FutureProducer;
use common::reload::{notify_config_changed, publish_config_changed, yaml_files, POLL_INTERVAL};
use contracts::MappingDef;
use serde_json::Value;

use crate::mapping::CompiledMapping;
use crate::{normalizer, Shutdown};

/// Aktualny zestaw skompilowanych mappingów (schemat -> mapping).
pub type MappingStore = Arc<ArcSwap<HashMap<String, Arc<CompiledMapping>>>>;

pub fn new_store() -> MappingStore {
    Arc::new(ArcSwap::from_pointee(HashMap::new()))
}

fn read_def(path: &Path) -> anyhow::Result<MappingDef> {
    Ok(serde_yaml::from_str(&std::fs::read_to_string(path)?)?)
}

/// Seed: wstawia mappingi z plików, nie nadpisując zmian zrobionych przez API.
pub async fn seed_mappings(pool: &PgPool, dir: &Path) -> anyhow::Result<()> {
    for path in yaml_files(dir) {
        match read_def(&path) {
            Ok(def) => {
                let json = serde_json::to_value(&def)?;
                sqlx_exec_seed(pool, &def.schema, json).await?;
            }
            Err(e) => tracing::warn!(file = %path.display(), error = %e, "invalid mapping file"),
        }
    }
    Ok(())
}

async fn sqlx_exec_seed(pool: &PgPool, schema: &str, def: Value) -> anyhow::Result<()> {
    common::db::sqlx::query(
        "INSERT INTO mappings (schema, def, version, updated_at) VALUES ($1, $2, 1, now()) ON CONFLICT (schema) DO NOTHING",
    )
    .bind(schema)
    .bind(def)
    .execute(pool)
    .await?;
    Ok(())
}

/// Wczytuje wszystkie mappingi z bazy do `store`; niekompilujące się są pomijane (warn).
pub async fn load_mappings(pool: &PgPool, store: &MappingStore) -> anyhow::Result<()> {
    let rows = common::db::sqlx::query("SELECT schema, def, version FROM mappings")
        .fetch_all(pool)
        .await?;
    let mut map = HashMap::new();
    for row in rows {
        let schema: String = row.get("schema");
        let def: Value = row.get("def");
        let version: i64 = row.get("version");
        let compiled = serde_json::from_value::<MappingDef>(def)
            .map_err(anyhow::Error::from)
            .and_then(|d| CompiledMapping::compile(d, version));
        match compiled {
            Ok(m) => {
                map.insert(schema, Arc::new(m));
            }
            Err(e) => tracing::warn!(%schema, error = %e, "mapping does not compile, skipped"),
        }
    }
    tracing::info!(count = map.len(), "mappings loaded");
    store.store(Arc::new(map));
    Ok(())
}

/// Sprawdza zmiany plików (po mtime) i wrzuca zmienione definicje do bazy.
async fn scan_files(pool: &PgPool, dir: &Path, seen: &mut HashMap<PathBuf, SystemTime>) {
    for path in yaml_files(dir) {
        let Ok(mtime) = std::fs::metadata(&path).and_then(|m| m.modified()) else {
            continue;
        };
        if seen.get(&path) == Some(&mtime) {
            continue;
        }
        seen.insert(path.clone(), mtime);
        let def = match read_def(&path) {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(file = %path.display(), error = %e, "invalid mapping file");
                continue;
            }
        };
        let Ok(json) = serde_json::to_value(&def) else {
            continue;
        };
        let res = common::db::sqlx::query(
            "INSERT INTO mappings (schema, def, version, updated_at) VALUES ($1, $2, 1, now())
             ON CONFLICT (schema) DO UPDATE SET def = EXCLUDED.def, version = mappings.version + 1, updated_at = now()
             WHERE mappings.def IS DISTINCT FROM EXCLUDED.def",
        )
        .bind(&def.schema)
        .bind(json)
        .execute(pool)
        .await;
        match res {
            Ok(r) if r.rows_affected() > 0 => {
                tracing::info!(schema = %def.schema, "mapping file changed, upserted");
                notify_config_changed(pool, "mappings").await;
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "mapping upsert failed"),
        }
    }
}

/// Seed + wstępne ładowanie + polling plików + nasłuch `config_changed` (przeładowanie i replay UNMAPPED).
/// Działa do sygnału zatrzymania.
pub async fn run_config(
    database_url: String,
    config_dir: String,
    pool: PgPool,
    producer: FutureProducer,
    store: MappingStore,
    mut shutdown: Shutdown,
) -> anyhow::Result<()> {
    let dir = Path::new(&config_dir).join("mappings");
    seed_mappings(&pool, &dir).await?;
    let mut seen: HashMap<PathBuf, SystemTime> = HashMap::new();
    for p in yaml_files(&dir) {
        if let Ok(m) = std::fs::metadata(&p).and_then(|m| m.modified()) {
            seen.insert(p, m);
        }
    }

    let mut listener = PgListener::connect(&database_url).await?;
    listener.listen("config_changed").await?;
    load_mappings(&pool, &store).await?;
    if let Err(e) = normalizer::replay_unmapped(&pool, &producer, &store).await {
        tracing::warn!(error = %e, "replay of unmapped events failed");
    }

    let mut tick = tokio::time::interval(POLL_INTERVAL);
    loop {
        tokio::select! {
            _ = shutdown.changed() => return Ok(()),
            _ = tick.tick() => scan_files(&pool, &dir, &mut seen).await,
            n = listener.recv() => match n {
                Ok(n) if n.payload() == "mappings" => {
                    if let Err(e) = load_mappings(&pool, &store).await {
                        tracing::warn!(error = %e, "mapping reload failed");
                        continue;
                    }
                    publish_config_changed(&producer, "mappings").await;
                    if let Err(e) = normalizer::replay_unmapped(&pool, &producer, &store).await {
                        tracing::warn!(error = %e, "replay of unmapped events failed");
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(error = %e, "config listener error");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    let _ = load_mappings(&pool, &store).await;
                }
            },
        }
    }
}
