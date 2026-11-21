//! router – binarka: reguły + routing (`alerts.changed` -> `deliveries.requested`, właściciel: P2).

use router::{new_store, run_config, run_router};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    common::init_tracing("router");
    let cfg = common::Config::from_env();
    let pool = common::db::connect_and_migrate(&cfg.database_url).await?;
    let producer = common::kafka::producer(&cfg.kafka_brokers)?;
    let store = new_store();
    let (tx, rx) = tokio::sync::watch::channel(false);

    let mut tasks = tokio::task::JoinSet::new();
    tasks.spawn(run_config(
        cfg.database_url.clone(),
        cfg.config_dir.clone(),
        pool,
        producer.clone(),
        store.clone(),
        rx.clone(),
    ));
    tasks.spawn(run_router(
        cfg.kafka_brokers.clone(),
        cfg.public_ui_url.clone(),
        producer,
        store,
        rx,
    ));

    tokio::select! {
        _ = common::shutdown_signal() => {}
        Some(res) = tasks.join_next() => {
            tracing::error!(?res, "router task exited unexpectedly");
        }
    }
    let _ = tx.send(true);
    while tasks.join_next().await.is_some() {}
    Ok(())
}
