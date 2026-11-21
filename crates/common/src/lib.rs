//! Wspólny runtime binarek (Faza 0, zamrożony jak `contracts`).
//!
//! - [`config`] – konfiguracja z env (te same nazwy zmiennych w całym compose),
//! - [`init_tracing`] – logi (`RUST_LOG`),
//! - [`db`] – pula Postgresa + migracje z `migrations/` (uruchamiane przy starcie),
//! - [`kafka`] – producent/konsument JSON z commitem offsetu po pracy,
//! - [`shutdown_signal`] – Ctrl+C / SIGTERM.

pub mod config;
#[cfg(feature = "db")]
pub mod db;
#[cfg(feature = "kafka")]
pub mod kafka;
pub mod reload;

pub use config::Config;

/// Inicjuje logowanie. Poziom z `RUST_LOG` (domyślnie `info`).
pub fn init_tracing(service: &str) {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).with_target(false).try_init();
    tracing::info!(service, "starting");
}

/// Czeka na Ctrl+C albo SIGTERM (docker stop).
pub async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
    tracing::info!("shutdown signal received");
}
