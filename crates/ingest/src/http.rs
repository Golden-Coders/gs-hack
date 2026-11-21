//! HTTP: `POST /ingest` i `POST /ingest/{schema}`.

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use common::kafka::rdkafka::producer::FutureProducer;
use serde_json::{json, Value};

use crate::{accept, parse_payload, ParseError};

#[derive(Clone)]
pub struct AppState {
    pub producer: FutureProducer,
}

#[derive(Clone)]
struct InnerState {
    producer: FutureProducer,
    token: Option<Arc<str>>,
}

pub fn app(state: AppState) -> Router {
    app_with_token(state, None)
}

pub fn app_with_token(state: AppState, token: Option<String>) -> Router {
    let inner = InnerState {
        producer: state.producer,
        token: token.filter(|t| !t.is_empty()).map(Arc::from),
    };
    Router::new()
        .route("/ingest", post(ingest))
        .route("/ingest/{schema}", post(ingest_schema))
        .with_state(inner)
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    let mut diff = (a.len() ^ b.len()) as u64;
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        diff |= u64::from(x ^ y);
    }
    std::hint::black_box(diff) == 0
}

fn check_auth(headers: &HeaderMap, expected: Option<&str>) -> Result<(), ApiError> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let provided = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    match provided {
        Some(token) if ct_eq(token.as_bytes(), expected.as_bytes()) => Ok(()),
        _ => Err(ApiError(
            StatusCode::UNAUTHORIZED,
            "unauthorized".into(),
            None,
        )),
    }
}

struct ApiError(StatusCode, String, Option<String>);

impl ApiError {
    fn bad(error: impl Into<String>, detail: Option<String>) -> Self {
        Self(StatusCode::BAD_REQUEST, error.into(), detail)
    }
}

impl From<ParseError> for ApiError {
    fn from(e: ParseError) -> Self {
        match e {
            ParseError::Unsupported(f) => Self(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                format!("unsupported format: {f}"),
                Some("supported: json, yaml".into()),
            ),
            ParseError::Invalid(d) => Self::bad("invalid payload", Some(d)),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut body = json!({"error": self.1});
        if let Some(d) = self.2 {
            body["detail"] = Value::String(d);
        }
        (self.0, Json(body)).into_response()
    }
}

async fn publish_raw(
    state: &InnerState,
    format: &str,
    schema: &str,
    payload: Value,
    source: String,
) -> Result<Response, ApiError> {
    let payload = parse_payload(format, &payload)?;
    match accept(&state.producer, format, schema, payload, source).await {
        Ok(id) => Ok((StatusCode::ACCEPTED, Json(json!({"event_id": id}))).into_response()),
        Err(e) => {
            tracing::error!(error = %e, "publishing to events.raw failed");
            Err(ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                "event bus unavailable".into(),
                Some(e.to_string()),
            ))
        }
    }
}

fn str_field<'a>(body: &'a Value, name: &str) -> Result<&'a str, ApiError> {
    body.get(name).and_then(Value::as_str).ok_or_else(|| {
        ApiError::bad(
            format!("missing field: {name}"),
            Some(format!("`{name}` must be a string")),
        )
    })
}

async fn ingest(
    State(state): State<InnerState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    check_auth(&headers, state.token.as_deref())?;
    let body: Value = serde_json::from_slice(&body)
        .map_err(|e| ApiError::bad("invalid json body", Some(e.to_string())))?;
    let format = str_field(&body, "format")?;
    let schema = str_field(&body, "schema")?;
    let payload = body
        .get("payload")
        .cloned()
        .ok_or_else(|| ApiError::bad("missing field: payload", None))?;
    publish_raw(&state, format, schema, payload, "http".into()).await
}

async fn ingest_schema(
    State(state): State<InnerState>,
    Path(schema): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    check_auth(&headers, state.token.as_deref())?;
    let ctype = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let ctype = ctype
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let (format, payload) = match ctype.as_str() {
        "application/json" => (
            "json",
            serde_json::from_slice(&body)
                .map_err(|e| ApiError::bad("invalid json body", Some(e.to_string())))?,
        ),
        "application/yaml" | "application/x-yaml" | "text/yaml" => {
            let text = String::from_utf8(body.to_vec())
                .map_err(|e| ApiError::bad("body is not utf-8", Some(e.to_string())))?;
            ("yaml", Value::String(text))
        }
        other => {
            return Err(ApiError(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                format!("unsupported content type: {other}"),
                Some("supported: application/json, application/yaml".into()),
            ))
        }
    };
    publish_raw(&state, format, &schema, payload, format!("http:{schema}")).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn auth_disabled_allows_requests_without_header() {
        let headers = HeaderMap::new();
        assert!(check_auth(&headers, None).is_ok());
    }

    #[test]
    fn auth_enabled_requires_exact_bearer_token() {
        let mut headers = HeaderMap::new();
        assert_eq!(
            check_auth(&headers, Some("secret-token")).unwrap_err().0,
            StatusCode::UNAUTHORIZED
        );

        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Basic secret-token"),
        );
        assert_eq!(
            check_auth(&headers, Some("secret-token")).unwrap_err().0,
            StatusCode::UNAUTHORIZED
        );

        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer wrong-token"),
        );
        assert_eq!(
            check_auth(&headers, Some("secret-token")).unwrap_err().0,
            StatusCode::UNAUTHORIZED
        );

        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer secret-toke"),
        );
        assert_eq!(
            check_auth(&headers, Some("secret-token")).unwrap_err().0,
            StatusCode::UNAUTHORIZED
        );

        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer secret-token"),
        );
        assert!(check_auth(&headers, Some("secret-token")).is_ok());
    }
}
