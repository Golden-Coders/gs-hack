//! ingest – HTTP /ingest + konsumpcja źródeł Kafki -> events.raw (właściciel: P1).
//!
//! SZKIELET Z FAZY 0: startuje, wykonuje migracje i czeka na sygnał zatrzymania.
//! Implementacja: `openspec/changes/alerting-service-mvp/tasks.md`, sekcje 2.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    common::init_tracing("ingest");
    let config = common::Config::from_env();
    let _pool = common::db::connect_and_migrate(&config.database_url).await?;

    tracing::info!("ingest skeleton ready; waiting for shutdown");
    common::shutdown_signal().await;
    Ok(())
}
