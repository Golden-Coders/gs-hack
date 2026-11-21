//! Domain HTTP route modules.

mod actions;
mod alerts;
mod deliveries;
mod mappings;
pub(crate) mod people;
mod rules;
mod runbooks;

use std::path::{Component, Path as FsPath, PathBuf};

use axum::Router;

use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .merge(alerts::router())
        .merge(mappings::router())
        .merge(rules::router())
        .merge(people::router())
        .merge(deliveries::router())
        .merge(runbooks::router())
        .merge(actions::router())
}

/// Resolves a relative config path safely inside the config directory whether the binary is
/// started from the repo root or `crates/api`. Rejects path traversal and non-relative paths.
pub(crate) fn resolve_config_path(config_dir: &str, rel: &str) -> Option<PathBuf> {
    if rel.is_empty() || rel.contains('\0') {
        return None;
    }
    let rel_path = FsPath::new(rel);
    if rel_path.components().any(|c| matches!(c, Component::ParentDir | Component::RootDir | Component::Prefix(_))) {
        return None;
    }

    let bases = [
        FsPath::new(config_dir),
        FsPath::new("config"),
        FsPath::new("../../config"),
    ];
    for base in bases {
        let candidate = base.join(rel_path);
        if candidate.exists() {
            let Ok(canon_base) = std::fs::canonicalize(base) else {
                continue;
            };
            let Ok(canon_cand) = std::fs::canonicalize(&candidate) else {
                continue;
            };
            if canon_cand.starts_with(&canon_base) {
                return Some(canon_cand);
            }
            return None;
        }
    }
    Some(FsPath::new(config_dir).join(rel_path))
}

#[cfg(test)]
pub(crate) mod test_util {
    use std::time::Duration;

    use common::db::sqlx::{
        self,
        types::{Json as SqlJson, Uuid},
    };
    use serde_json::{json, Value};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    use crate::AppState;

    pub struct Server {
        pub addr: std::net::SocketAddr,
        pub st: AppState,
        pub alerts: std::sync::Mutex<Vec<Uuid>>,
    }

    pub async fn server() -> Server {
        let st = AppState::connect(common::Config::from_env()).await.expect("connect");
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = crate::router(st.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Server { addr, st, alerts: Default::default() }
    }

    impl Server {
        /// Sends an HTTP request with `Authorization: Bearer <api_token>`.
        pub async fn http(&self, method: &str, path: &str, headers: &[(&str, &str)], body: Option<&str>) -> (u16, Value) {
            let auth = format!("Bearer {}", self.st.config.api_token);
            let mut all = vec![("Authorization", auth.as_str())];
            all.extend_from_slice(headers);
            let (status, _, json) = self.http_raw(method, path, &all, body).await;
            (status, json)
        }

        /// Sends a raw HTTP/1.1 request without adding default headers; returns status, lowercased headers, and JSON body.
        pub async fn http_raw(&self, method: &str, path: &str, headers: &[(&str, &str)], body: Option<&str>) -> (u16, String, Value) {
            let mut req = format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n");
            for (k, v) in headers {
                req.push_str(&format!("{k}: {v}\r\n"));
            }
            if let Some(b) = body {
                req.push_str(&format!("Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{b}", b.len()));
            } else {
                req.push_str("\r\n");
            }
            let mut s = TcpStream::connect(self.addr).await.unwrap();
            s.write_all(req.as_bytes()).await.unwrap();
            let mut buf = Vec::new();
            s.read_to_end(&mut buf).await.unwrap();
            let text = String::from_utf8(buf).unwrap();
            let (head, body) = text.split_once("\r\n\r\n").unwrap();
            let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
            let json = serde_json::from_str(body.trim()).unwrap_or(Value::Null);
            (status, head.to_lowercase(), json)
        }

        /// Reads only status and headers (for non-terminating SSE streams).
        pub async fn head_only(&self, path: &str) -> (u16, String) {
            let req = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n");
            let mut s = TcpStream::connect(self.addr).await.unwrap();
            s.write_all(req.as_bytes()).await.unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 1024];
            while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = tokio::time::timeout(Duration::from_secs(5), s.read(&mut chunk)).await.expect("timeout").unwrap();
                assert!(n > 0, "połączenie zamknięte przed końcem nagłówków");
                buf.extend_from_slice(&chunk[..n]);
            }
            let head = String::from_utf8_lossy(&buf).to_lowercase();
            (head.split_whitespace().nth(1).unwrap().parse().unwrap(), head)
        }

        /// Deletes alerts inserted via `insert_alert`.
        pub async fn cleanup(&self) {
            let ids = self.alerts.lock().unwrap().clone();
            sqlx::query("DELETE FROM alert_episodes WHERE alert_id = ANY($1)").bind(&ids).execute(&self.st.db).await.unwrap();
            sqlx::query("DELETE FROM alerts WHERE id = ANY($1)").bind(&ids).execute(&self.st.db).await.unwrap();
        }

        pub async fn insert_alert(&self, fp: &str, state: &str, age_days: i32) -> Uuid {
            let id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO alerts (id, fingerprint, state, severity, title, schema, episode, count, first_seen, last_seen, \
                 last_event, acked_by, updated_at) VALUES ($1, $2, $3, 'warning', 'tytul', 'test', 2, 5, now(), now(), $4, $5, \
                 now() - make_interval(days => $6))",
            )
            .bind(id)
            .bind(fp)
            .bind(state)
            .bind(SqlJson(json!({"title": "tytul"})))
            .bind(SqlJson(json!({"type": "user", "person_id": "ala"})))
            .bind(age_days)
            .execute(&self.st.db)
            .await
            .unwrap();
            self.alerts.lock().unwrap().push(id);
            id
        }
    }

    pub fn uniq(prefix: &str) -> String {
        format!("{prefix}-{}", Uuid::new_v4())
    }
}
