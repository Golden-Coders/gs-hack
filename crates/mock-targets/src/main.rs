//! mock-targets – symulowane usługi docelowe akcji runbooków (właściciel: P3, zadanie 10.4).
//!
//! Każde żądanie poza `/health` i `/calls` jest przyjmowane i logowane, więc działają
//! `/retry_workflow`, `/restart_service`, `/open_ticket`, `/counterparty-service/health`,
//! `/cache/{service}/flush` i każdy nowy endpoint dodany akcją z UI.
//!
//! Sterowanie na potrzeby demo/testów (query): `?status=500` – zwróć ten kod,
//! `?delay_ms=10000` – odpowiedz z opóźnieniem (scenariusz „Wolna usługa”).
//! Powtórzony `Idempotency-Key` nie „wykonuje” akcji drugi raz: wpis ma `replayed: true`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use serde_json::{json, Value};

/// Log w pamięci: ostatnie `MAX_CALLS` wywołań i ograniczone `seen_keys` (`MAX_SEEN_KEYS`).
const MAX_CALLS: usize = 1000;
const MAX_SEEN_KEYS: usize = MAX_CALLS * 2;

#[derive(Default)]
struct SeenKeys {
    set: HashSet<String>,
    order: VecDeque<String>,
}

impl SeenKeys {
    /// Zwraca `true`, jeśli klucz był nowy (nie był wcześniej widziany).
    fn insert(&mut self, key: String) -> bool {
        if !self.set.insert(key.clone()) {
            return false;
        }
        self.order.push_back(key);
        while self.order.len() > MAX_SEEN_KEYS {
            if let Some(oldest) = self.order.pop_front() {
                self.set.remove(&oldest);
            }
        }
        true
    }

    fn clear(&mut self) {
        self.set.clear();
        self.order.clear();
    }
}

#[derive(Clone, Default)]
struct AppState {
    calls: Arc<Mutex<Vec<Call>>>,
    seen_keys: Arc<Mutex<SeenKeys>>,
}

#[derive(Clone, Serialize)]
struct Call {
    at: chrono::DateTime<chrono::Utc>,
    method: String,
    path: String,
    idempotency_key: Option<String>,
    body: Value,
    status: u16,
    replayed: bool,
}

fn app() -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/calls", get(list_calls).delete(clear_calls))
        .fallback(record)
        .with_state(AppState::default())
}

async fn list_calls(State(s): State<AppState>) -> Json<Vec<Call>> {
    Json(s.calls.lock().unwrap_or_else(|e| e.into_inner()).clone())
}

async fn clear_calls(State(s): State<AppState>) -> StatusCode {
    s.calls.lock().unwrap_or_else(|e| e.into_inner()).clear();
    s.seen_keys.lock().unwrap_or_else(|e| e.into_inner()).clear();
    StatusCode::NO_CONTENT
}

async fn record(
    State(s): State<AppState>,
    method: Method,
    uri: Uri,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: Bytes,
) -> (StatusCode, Json<Value>) {
    if let Some(ms) = query.get("delay_ms").and_then(|v| v.parse::<u64>().ok()) {
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }
    let status = query
        .get("status")
        .and_then(|v| v.parse::<u16>().ok())
        .and_then(|c| StatusCode::from_u16(c).ok())
        .unwrap_or(StatusCode::OK);
    let idempotency_key = headers.get("idempotency-key").and_then(|v| v.to_str().ok()).map(str::to_string);
    let replayed = idempotency_key
        .as_ref()
        .is_some_and(|k| !s.seen_keys.lock().unwrap_or_else(|e| e.into_inner()).insert(k.clone()));
    let body = serde_json::from_slice(&body).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&body).into_owned()));
    let path = uri.path().to_string();
    tracing::info!(%method, path, replayed, status = status.as_u16(), "mock call");

    let mut calls = s.calls.lock().unwrap_or_else(|e| e.into_inner());
    calls.push(Call {
        at: chrono::Utc::now(),
        method: method.to_string(),
        path: path.clone(),
        idempotency_key,
        body,
        status: status.as_u16(),
        replayed,
    });
    if calls.len() > MAX_CALLS {
        let excess = calls.len() - MAX_CALLS;
        calls.drain(..excess);
    }
    (status, Json(json!({ "ok": status.is_success(), "path": path, "replayed": replayed })))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    common::init_tracing("mock-targets");
    let port = common::Config::port("MOCK_TARGETS", 8090);
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    tracing::info!(port, "mock-targets listening");
    axum::serve(listener, app()).with_graceful_shutdown(common::shutdown_signal()).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn spawn() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app()).await.unwrap() });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn retry_workflow_is_logged_once_per_idempotency_key() {
        let base = spawn().await;
        let client = reqwest::Client::new();
        for _ in 0..2 {
            let resp = client
                .post(format!("{base}/retry_workflow"))
                .header("Idempotency-Key", "step:run-1:s2:0")
                .json(&json!({ "workflowId": "WF-20261023-00042" }))
                .send()
                .await
                .unwrap();
            assert!(resp.status().is_success());
        }
        let calls: Vec<Value> = client.get(format!("{base}/calls")).send().await.unwrap().json().await.unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0]["path"], "/retry_workflow");
        assert_eq!(calls[0]["body"]["workflowId"], "WF-20261023-00042");
        assert_eq!(calls[0]["replayed"], false);
        assert_eq!(calls[1]["replayed"], true);
    }

    #[tokio::test]
    async fn status_override() {
        let base = spawn().await;
        let resp = reqwest::get(format!("{base}/counterparty-service/health?status=503")).await.unwrap();
        assert_eq!(resp.status(), 503);
    }

    #[test]
    fn seen_keys_is_bounded_and_evicts_oldest() {
        let mut seen = SeenKeys::default();
        for i in 0..(MAX_SEEN_KEYS + 10) {
            assert!(seen.insert(format!("k-{i}")));
        }
        assert_eq!(seen.set.len(), MAX_SEEN_KEYS);
        assert_eq!(seen.order.len(), MAX_SEEN_KEYS);
        // Oldest keys were evicted, newest are still deduplicated.
        assert!(!seen.insert(format!("k-{}", MAX_SEEN_KEYS + 9)));
        assert!(seen.insert("k-0".into()));
    }
}
