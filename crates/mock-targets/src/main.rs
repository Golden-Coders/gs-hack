//! mock-targets – symulowane usługi docelowe akcji runbooków (właściciel: P3).
//!
//! SZKIELET Z FAZY 0: tylko `GET /health`. Implementacja: `tasks.md` zadanie 10.4.

use axum::{routing::get, Router};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    common::init_tracing("mock-targets");
    let port = common::Config::port("MOCK_TARGETS", 8090);
    let app = Router::new().route("/health", get(|| async { "ok" }));
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    tracing::info!(port, "mock-targets listening");
    axum::serve(listener, app).with_graceful_shutdown(common::shutdown_signal()).await?;
    Ok(())
}
