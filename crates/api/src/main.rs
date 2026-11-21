//! REST + SSE API gateway for the AlertHub UI.

mod auth;
mod error;
mod routes;
mod rules_eval;
mod state;
mod stream;

use axum::{
    extract::{Request, State},
    http::{header, HeaderName, HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use tower_http::cors::{AllowOrigin, CorsLayer};

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
    axum::serve(listener, app)
        .with_graceful_shutdown(common::shutdown_signal())
        .await?;
    Ok(())
}

fn cors_layer(public_ui_url: &str) -> CorsLayer {
    let mut origins: Vec<HeaderValue> = Vec::new();
    for raw in [
        public_ui_url.trim_end_matches('/'),
        "http://localhost:5173",
        "http://127.0.0.1:5173",
    ] {
        if raw.is_empty() {
            continue;
        }
        if let Ok(val) = HeaderValue::from_str(raw) {
            if !origins.contains(&val) {
                origins.push(val);
            }
        }
    }
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_credentials(true)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::ACCEPT,
            header::CACHE_CONTROL,
            HeaderName::from_static("x-person-id"),
        ])
}

fn router(state: AppState) -> Router {
    let cors = cors_layer(&state.config.public_ui_url);
    Router::new()
        .route("/stream", get(stream::sse))
        .merge(auth::protected_router())
        .merge(routes::router())
        .route_layer(middleware::from_fn_with_state(state.clone(), require_token))
        .merge(auth::public_router())
        .route("/health", get(|| async { "ok" }))
        .layer(cors)
        .with_state(state)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    auth::constant_time_eq(a, b)
}

/// Authenticates requests via:
/// 1. Server-issued operator session (`Authorization: Bearer <session>` or `HttpOnly` cookie `alerthub_session`).
/// 2. Per-operator token (`API_USER_TOKENS`).
/// 3. Shared service `API_TOKEN` (`Authorization: Bearer` or `?token=` on `/stream` only),
///    optionally bound to a reverse-proxy identity header (`TRUSTED_PROXY_AUTH_HEADER`).
async fn require_token(State(state): State<AppState>, mut req: Request, next: Next) -> Response {
    let bearer = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string);
    let cookie_tok = auth::extract_cookie(req.headers(), auth::SESSION_COOKIE_NAME);

    let stream_query_tok: Option<String> = (req.uri().path() == "/stream")
        .then(|| {
            req.uri().query().and_then(|q| {
                q.split('&')
                    .find_map(|kv| kv.strip_prefix("token=").map(str::to_string))
            })
        })
        .flatten();
    for candidate in [
        bearer.as_deref(),
        cookie_tok.as_deref(),
        stream_query_tok.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(person_id) = state.auth.lookup_session(candidate) {
            req.extensions_mut().insert(auth::AuthenticatedOperator {
                person_id: Some(person_id),
                method: auth::AuthMethod::Session,
                session_token: Some(candidate.to_string()),
            });
            return next.run(req).await;
        }
    }

    if let Some(tok) = bearer.as_deref() {
        if let Some(person_id) = state.auth.lookup_user_token(tok) {
            req.extensions_mut().insert(auth::AuthenticatedOperator {
                person_id: Some(person_id),
                method: auth::AuthMethod::UserToken,
                session_token: None,
            });
            return next.run(req).await;
        }
    }

    let expected = state.config.api_token.as_str();
    if expected.is_empty() {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "unauthorized" })),
        )
            .into_response();
    }
    let expected_bytes = expected.as_bytes();
    let header_ok = bearer
        .as_deref()
        .is_some_and(|t| constant_time_eq(t.as_bytes(), expected_bytes));
    let query_ok = req.uri().path() == "/stream"
        && req.uri().query().is_some_and(|q| {
            q.split('&').any(|kv| {
                kv.strip_prefix("token=")
                    .is_some_and(|t| constant_time_eq(t.as_bytes(), expected_bytes))
            })
        });
    if header_ok || query_ok {
        let proxy_person = state.auth.trusted_proxy_header().and_then(|hdr| {
            req.headers()
                .get(hdr)
                .and_then(|v| v.to_str().ok())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        });
        let method = if proxy_person.is_some() {
            auth::AuthMethod::TrustedProxy
        } else {
            auth::AuthMethod::ServiceToken
        };
        req.extensions_mut().insert(auth::AuthenticatedOperator {
            person_id: proxy_person,
            method,
            session_token: None,
        });
        next.run(req).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "unauthorized" })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_works() {
        assert!(constant_time_eq(b"secret-token", b"secret-token"));
        assert!(!constant_time_eq(b"secret-token", b"secret-tokeN"));
        assert!(!constant_time_eq(b"secret-token", b"short"));
        assert!(constant_time_eq(b"", b""));
    }
}
