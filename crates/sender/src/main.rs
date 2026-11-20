//! sender – adaptery kanałów, retry, fallback, DLQ (właściciel: P2).
//!
//! SZKIELET Z FAZY 0: startuje, wykonuje migracje i czeka na sygnał zatrzymania.
//! Implementacja: `openspec/changes/alerting-service-mvp/tasks.md`, sekcje 9.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    common::init_tracing("sender");
    let config = common::Config::from_env();
    let _pool = common::db::connect_and_migrate(&config.database_url).await?;

    tracing::info!("sender skeleton ready; waiting for shutdown");
    common::shutdown_signal().await;
    Ok(())
}
