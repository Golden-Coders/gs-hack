//! ingest – HTTP /ingest + konsumpcja źródeł Kafki -> events.raw (właściciel: P1).

use std::sync::Arc;

use ingest::{http, sources};

/// Domyślny maks. rozmiar puli Postgresa (nadpisywany przez `DB_MAX_CONNECTIONS`, zob. docs/LOADTEST.md).
const DB_POOL_DEFAULT: u32 = 2;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    common::init_tracing("ingest");
    let config = common::Config::from_env();
    let pool = common::db::connect_and_migrate_with(&config.database_url, DB_POOL_DEFAULT).await?;
    let producer = common::kafka::producer(&config.kafka_brokers)?;

    let ctx = Arc::new(sources::Ctx {
        brokers: config.kafka_brokers.clone(),
        pool,
        producer: producer.clone(),
    });
    let scanner = tokio::spawn(sources::run_scanner(
        ctx,
        format!("{}/sources", config.config_dir),
    ));

    let ingest_token = std::env::var("INGEST_TOKEN").ok().filter(|t| !t.is_empty());
    if ingest_token.is_some() {
        tracing::info!("ingest bearer token authentication enabled");
    }

    let port = common::Config::port("INGEST", 8081);
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    tracing::info!(port, "ingest listening");
    axum::serve(
        listener,
        http::app_with_token(http::AppState { producer }, ingest_token),
    )
    .with_graceful_shutdown(common::shutdown_signal())
    .await?;
    scanner.abort();
    Ok(())
}
