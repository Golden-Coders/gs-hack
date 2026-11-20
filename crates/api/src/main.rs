//! api – gateway REST + SSE dla UI (design D13).
//!
//! TEN PLIK JEST ZAMROŻONY (Faza 0). Routy dodaje się wyłącznie w module
//! własnego strumienia: `p1.rs` (alerty, mappingi, ingest), `p2.rs` (reguły,
//! osoby, dostarczenia, mocki), `p3.rs` (runbooki, akcje, zatwierdzenia).
//! Wspólne: `state.rs` (stan aplikacji), `/health`, `/stream` (SSE).

mod p1;
mod p2;
mod p3;
mod state;
mod stream;

use axum::{routing::get, Router};
use tower_http::cors::CorsLayer;

pub use state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    common::init_tracing("api");
    let config = common::Config::from_env();
    let state = AppState::connect(config).await?;

    let app = router(state);

    let port = common::Config::port("API", 8080);
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    tracing::info!(port, "api listening");
    axum::serve(listener, app).with_graceful_shutdown(common::shutdown_signal()).await?;
    Ok(())
}

/// Lista modułów – ZAMROŻONA. Nowe endpointy dodajecie w `pN::routes()`.
fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/stream", get(stream::sse))
        .merge(p1::routes())
        .merge(p2::routes())
        .merge(p3::routes())
        .layer(CorsLayer::permissive())
        .with_state(state)
}
