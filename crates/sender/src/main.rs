//! sender – adaptery kanałów, retry, fallback, DLQ (właściciel: P2).

use std::sync::Arc;

use sender::{run_sender, run_sender_retry, PgKafkaBackend, SenderConfig, SenderEngine};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    common::init_tracing("sender");
    let config = common::Config::from_env();
    let pool = common::db::connect_and_migrate(&config.database_url).await?;
    let producer = common::kafka::producer(&config.kafka_brokers)?;

    let backend = Arc::new(PgKafkaBackend::new(pool, producer));
    let sender_cfg = SenderConfig::from_env();
    let engine = SenderEngine::new(backend, &config.smtp_host, config.smtp_port, sender_cfg);

    let (tx, rx) = tokio::sync::watch::channel(false);
    let mut tasks = tokio::task::JoinSet::new();

    tasks.spawn(run_sender(
        config.kafka_brokers.clone(),
        engine.clone(),
        rx.clone(),
    ));
    tasks.spawn(run_sender_retry(config.kafka_brokers.clone(), engine, rx));

    tokio::select! {
        _ = common::shutdown_signal() => {}
        Some(res) = tasks.join_next() => {
            tracing::error!(?res, "sender task exited unexpectedly");
        }
    }

    let _ = tx.send(true);
    while tasks.join_next().await.is_some() {}
    Ok(())
}
