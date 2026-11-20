//! HTTP: `POST /ingest` i `POST /ingest/{schema}`.

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

pub fn app(state: AppState) -> Router {
    Router::new().route("/ingest", post(ingest)).route("/ingest/{schema}", post(ingest_schema)).with_state(state)
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
            ParseError::Unsupported(f) => Self(StatusCode::UNSUPPORTED_MEDIA_TYPE, format!("unsupported format: {f}"), Some("supported: json, yaml".into())),
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

async fn publish_raw(state: &AppState, format: &str, schema: &str, payload: Value, source: String) -> Result<Response, ApiError> {
    let payload = parse_payload(format, &payload)?;
    match accept(&state.producer, format, schema, payload, source).await {
        Ok(id) => Ok((StatusCode::ACCEPTED, Json(json!({"event_id": id}))).into_response()),
        Err(e) => {
            tracing::error!(error = %e, "publishing to events.raw failed");
            Err(ApiError(StatusCode::SERVICE_UNAVAILABLE, "event bus unavailable".into(), Some(e.to_string())))
        }
    }
}

fn str_field<'a>(body: &'a Value, name: &str) -> Result<&'a str, ApiError> {
    body.get(name).and_then(Value::as_str).ok_or_else(|| ApiError::bad(format!("missing field: {name}"), Some(format!("`{name}` must be a string"))))
}

async fn ingest(State(state): State<AppState>, body: Bytes) -> Result<Response, ApiError> {
    let body: Value = serde_json::from_slice(&body).map_err(|e| ApiError::bad("invalid json body", Some(e.to_string())))?;
    let format = str_field(&body, "format")?;
    let schema = str_field(&body, "schema")?;
    let payload = body.get("payload").cloned().ok_or_else(|| ApiError::bad("missing field: payload", None))?;
    publish_raw(&state, format, schema, payload, "http".into()).await
}

async fn ingest_schema(State(state): State<AppState>, Path(schema): Path<String>, headers: HeaderMap, body: Bytes) -> Result<Response, ApiError> {
    let ctype = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("");
    let ctype = ctype.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    let (format, payload) = match ctype.as_str() {
        "application/json" => ("json", serde_json::from_slice(&body).map_err(|e| ApiError::bad("invalid json body", Some(e.to_string())))?),
        "application/yaml" | "application/x-yaml" | "text/yaml" => {
            let text = String::from_utf8(body.to_vec()).map_err(|e| ApiError::bad("body is not utf-8", Some(e.to_string())))?;
            ("yaml", Value::String(text))
        }
        other => return Err(ApiError(StatusCode::UNSUPPORTED_MEDIA_TYPE, format!("unsupported content type: {other}"), Some("supported: application/json, application/yaml".into()))),
    };
    publish_raw(&state, format, &schema, payload, format!("http:{schema}")).await
}
