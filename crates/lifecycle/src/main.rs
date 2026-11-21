//! lifecycle – cienka binarka: normalizer + dedup + konfiguracja mappingów (właściciel: P1).

use lifecycle::{config, run_config, run_dedup, run_normalizer};

/// Domyślny maks. rozmiar puli Postgresa (nadpisywany przez `DB_MAX_CONNECTIONS`, zob. docs/LOADTEST.md).
const DB_POOL_DEFAULT: u32 = 16;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    common::init_tracing("lifecycle");
    let cfg = common::Config::from_env();
    let pool = common::db::connect_and_migrate_with(&cfg.database_url, DB_POOL_DEFAULT).await?;
    let producer = common::kafka::producer(&cfg.kafka_brokers)?;
    let store = config::new_store();
    let (tx, rx) = tokio::sync::watch::channel(false);

    let mut tasks = tokio::task::JoinSet::new();
    tasks.spawn(run_config(cfg.database_url.clone(), cfg.config_dir.clone(), pool.clone(), producer.clone(), store.clone(), rx.clone()));
    tasks.spawn(run_normalizer(cfg.kafka_brokers.clone(), pool.clone(), producer.clone(), store, rx.clone()));
    tasks.spawn(run_dedup(cfg.kafka_brokers.clone(), pool, producer, rx));

    tokio::select! {
        _ = common::shutdown_signal() => {}
        Some(res) = tasks.join_next() => {
            tracing::error!(?res, "lifecycle task exited unexpectedly");
        }
    }
    let _ = tx.send(true);
    while tasks.join_next().await.is_some() {}
    Ok(())
}
