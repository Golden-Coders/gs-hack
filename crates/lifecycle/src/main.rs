//! lifecycle – normalizer (events.raw -> events.normalized) + dedup i stan alertów (właściciel: P1).
//!
//! SZKIELET Z FAZY 0: startuje, wykonuje migracje i czeka na sygnał zatrzymania.
//! Implementacja: `openspec/changes/alerting-service-mvp/tasks.md`, sekcje 3, 4.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    common::init_tracing("lifecycle");
    let config = common::Config::from_env();
    let _pool = common::db::connect_and_migrate(&config.database_url).await?;
    // Ewaluator warunków z crate `condition` (P2) – na razie zaślepka.
    let _ = condition::compile;
    tracing::info!("lifecycle skeleton ready; waiting for shutdown");
    common::shutdown_signal().await;
    Ok(())
}
