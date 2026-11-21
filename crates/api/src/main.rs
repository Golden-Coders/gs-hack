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

use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
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
        .route("/stream", get(stream::sse))
        .merge(p1::routes())
        .merge(p2::routes())
        .merge(p3::routes())
        // Wszystko powyżej wymaga tokenu; `/health` (niżej) jest publiczne.
        .route_layer(middleware::from_fn_with_state(state.clone(), require_token))
        .route("/health", get(|| async { "ok" }))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

/// `Authorization: Bearer <API_TOKEN>`; dla SSE także `?token=<API_TOKEN>`, bo `EventSource` nie wysyła nagłówków.
async fn require_token(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let expected = state.config.api_token.as_str();
    let header_ok = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|t| t == expected);
    let query_ok = req.uri().query().is_some_and(|q| q.split('&').any(|kv| kv.strip_prefix("token=") == Some(expected)));
    if header_ok || query_ok {
        next.run(req).await
    } else {
        (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "unauthorized" }))).into_response()
    }
}
