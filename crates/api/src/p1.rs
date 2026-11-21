//! Routy strumienia P1: alerty (lista, szczegóły, ack/resolve), mappingi,
//! UNMAPPED, błędy ingestu. Właściciel: P1. Zob. `contracts/openapi.yaml` (tag `p1`).

use axum::{
    extract::{rejection::JsonRejection, Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use common::db::sqlx::{
    self,
    types::{chrono::DateTime, chrono::Utc, Json as SqlJson, Uuid},
    Row,
};
use contracts::{formats, topics, Actor, AlertCommand, AlertCommandKind, CanonicalEvent, Envelope, MappingDef, Recipient, RuleDef, Severity};
use lifecycle::mapping::{parse_payload, CompiledMapping};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::AppState;

type ApiResult<T> = Result<T, (StatusCode, Json<Value>)>;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/ingest/errors", get(ingest_errors))
        .route("/alerts", get(list_alerts))
        .route("/alerts/{alert_id}", get(get_alert))
        .route("/alerts/{alert_id}/episodes", get(episodes))
        .route("/alerts/{alert_id}/ack", post(ack))
        .route("/alerts/{alert_id}/resolve", post(resolve))
        .route("/mappings", get(list_mappings))
        .route("/mappings/preview", post(preview))
        .route("/mappings/{schema}", get(get_mapping).put(put_mapping).delete(delete_mapping))
        .route("/unmapped", get(unmapped))
}

/// Jedyny pomocnik błędów: `{"error": ..., "detail": ...}`.
fn err(status: StatusCode, error: &str, detail: impl ToString) -> (StatusCode, Json<Value>) {
    (status, Json(json!({ "error": error, "detail": detail.to_string() })))
}

fn db_err(e: sqlx::Error) -> (StatusCode, Json<Value>) {
    tracing::error!(error = %e, "db error");
    err(StatusCode::INTERNAL_SERVER_ERROR, "database error", e)
}

fn parse_id(s: &str) -> ApiResult<Uuid> {
    Uuid::parse_str(s).map_err(|e| err(StatusCode::BAD_REQUEST, "invalid alert_id", e))
}

fn opt_json(r: &sqlx::postgres::PgRow, col: &str) -> Value {
    r.get::<Option<SqlJson<Value>>, _>(col).map_or(Value::Null, |j| j.0)
}

async fn ingest_errors(State(st): State<AppState>) -> ApiResult<Json<Value>> {
    let rows = sqlx::query("SELECT source, error, sample, at FROM ingest_errors ORDER BY at DESC LIMIT 100")
        .fetch_all(&st.db)
        .await
        .map_err(db_err)?;
    let out: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "source": r.get::<String, _>("source"),
                "error": r.get::<String, _>("error"),
                "sample": r.get::<Option<String>, _>("sample"),
                "at": r.get::<DateTime<Utc>, _>("at"),
            })
        })
        .collect();
    Ok(Json(json!(out)))
}

const ALERT_COLS: &str = "id, fingerprint, state, severity, title, schema, count::bigint AS count, episode::int AS episode, first_seen, last_seen, \
                          last_event, acked_by, resolved_by";

struct CompiledRule {
    def: RuleDef,
    condition: condition::Compiled,
    teams: Vec<String>,
}

impl CompiledRule {
    fn from_def(def: RuleDef) -> Option<Self> {
        if !def.enabled {
            return None;
        }
        let condition = condition::compile(&def.condition).ok()?;
        let mut teams: Vec<String> = def.team.iter().cloned().collect();
        for r in &def.recipients {
            if let Recipient::Team(t) = r {
                if !teams.contains(t) {
                    teams.push(t.clone());
                }
            }
        }
        Some(Self { def, condition, teams })
    }
}

async fn load_enabled_rules(db: &sqlx::PgPool) -> Vec<CompiledRule> {
    match sqlx::query("SELECT def, enabled FROM rules WHERE enabled = TRUE ORDER BY id")
        .fetch_all(db)
        .await
    {
        Ok(rows) => rows
            .iter()
            .filter_map(|row| {
                let mut def: RuleDef = serde_json::from_value(row.get("def")).ok()?;
                def.enabled = row.get("enabled");
                CompiledRule::from_def(def)
            })
            .collect(),
        Err(e) => {
            tracing::warn!(error = %e, "p1: loading rules failed, returning unenriched alerts");
            Vec::new()
        }
    }
}

fn severity_to_str(s: Severity) -> &'static str {
    match s {
        Severity::Info => "info",
        Severity::Warning => "warning",
        Severity::Error => "error",
        Severity::Critical => "critical",
    }
}

fn resolve_path<'a>(ctx: &'a Value, path: &str) -> Value {
    let mut cur = ctx;
    for seg in path.split('.') {
        match cur {
            Value::Object(map) => match map.get(seg) {
                Some(next) => cur = next,
                None => return Value::Null,
            },
            Value::Array(arr) => match seg.parse::<usize>().ok().and_then(|i| arr.get(i)) {
                Some(next) => cur = next,
                None => return Value::Null,
            },
            _ => return Value::Null,
        }
    }
    cur.clone()
}

struct AlertEnrichment {
    rule_ids: Vec<String>,
    teams: Vec<String>,
    severity: String,
    title: String,
    color: Option<String>,
    body: String,
    fields: Vec<Value>,
}

/// D10 (hardening 6.1): wylicza `rule_ids`, `teams`, nadpisane `severity` oraz prezentację
/// (`title`, `body`, `fields`, `color`) na podstawie włączonych reguł P2 (tylko odczyt).
fn enrich_alert(base_severity: &str, base_title: &str, last_event: &Value, rules: &[CompiledRule]) -> AlertEnrichment {
    let mut out = AlertEnrichment {
        rule_ids: Vec::new(),
        teams: Vec::new(),
        severity: base_severity.to_string(),
        title: base_title.to_string(),
        color: None,
        body: String::new(),
        fields: Vec::new(),
    };
    let Ok(event) = serde_json::from_value::<CanonicalEvent>(last_event.clone()) else {
        return out;
    };
    let ctx = condition::event_context(&event);
    let mut max_override: Option<Severity> = None;
    let mut title_set = false;
    let mut body_set = false;
    let mut seen_fields: Vec<String> = Vec::new();

    for rule in rules {
        if !rule.condition.evaluate(&ctx) {
            continue;
        }
        out.rule_ids.push(rule.def.id.clone());
        for t in &rule.teams {
            if !out.teams.contains(t) {
                out.teams.push(t.clone());
            }
        }
        if let Some(ov) = rule.def.severity_override {
            max_override = Some(match max_override {
                Some(cur) => cur.max(ov),
                None => ov,
            });
        }
        if !title_set {
            if let Some(tpl) = rule.def.presentation.title.as_deref().filter(|s| !s.trim().is_empty()) {
                if let Ok(rendered) = condition::render_template(tpl, &ctx) {
                    if !rendered.trim().is_empty() {
                        out.title = rendered;
                        title_set = true;
                    }
                }
            }
        }
        if out.color.is_none() {
            if let Some(c) = rule.def.presentation.color.as_deref().filter(|s| !s.trim().is_empty()) {
                out.color = Some(c.to_string());
            }
        }
        if !body_set {
            if let Some(tpl) = rule.def.presentation.body.as_deref().filter(|s| !s.trim().is_empty()) {
                if let Ok(rendered) = condition::render_template(tpl, &ctx) {
                    out.body = rendered;
                    body_set = true;
                }
            }
        }
        for path in &rule.def.presentation.fields {
            if seen_fields.contains(path) {
                continue;
            }
            seen_fields.push(path.clone());
            let label = path.strip_prefix("raw.").unwrap_or(path);
            let value = resolve_path(&ctx, path);
            let highlight = rule.def.presentation.highlight.get(path).cloned();
            out.fields.push(json!({
                "path": path,
                "label": label,
                "value": value,
                "highlight": highlight,
            }));
        }
    }

    if let Some(ov) = max_override {
        out.severity = severity_to_str(ov).to_string();
    }
    out
}

fn summary(r: &sqlx::postgres::PgRow, rules: &[CompiledRule]) -> (serde_json::Map<String, Value>, AlertEnrichment) {
    let base_sev = r.get::<String, _>("severity");
    let base_title = r.get::<String, _>("title");
    let last_event = opt_json(r, "last_event");
    let enriched = enrich_alert(&base_sev, &base_title, &last_event, rules);
    let v = json!({
        "id": r.get::<Uuid, _>("id"),
        "fingerprint": r.get::<String, _>("fingerprint"),
        "state": r.get::<String, _>("state"),
        "severity": enriched.severity,
        "title": enriched.title,
        "schema": r.get::<String, _>("schema"),
        "count": r.get::<i64, _>("count"),
        "episode": r.get::<i32, _>("episode"),
        "first_seen": r.get::<DateTime<Utc>, _>("first_seen"),
        "last_seen": r.get::<DateTime<Utc>, _>("last_seen"),
        "rule_ids": enriched.rule_ids,
        "teams": enriched.teams,
        "color": enriched.color,
    });
    match v {
        Value::Object(m) => (m, enriched),
        _ => unreachable!(),
    }
}

#[derive(Deserialize)]
struct AlertsQuery {
    state: Option<String>,
    since: Option<DateTime<Utc>>,
    limit: Option<i64>,
    team: Option<String>,
}

async fn list_alerts(State(st): State<AppState>, Query(q): Query<AlertsQuery>) -> ApiResult<Json<Value>> {
    let limit = q.limit.unwrap_or(100).clamp(1, 1000);
    let team_filter = q.team.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let rules = load_enabled_rules(&st.db).await;

    let rows = if team_filter.is_some() {
        let sql = format!(
            "SELECT {ALERT_COLS} FROM alerts WHERE ($1::text IS NULL OR state = $1) \
             AND ($2::timestamptz IS NULL OR updated_at > $2) ORDER BY updated_at DESC"
        );
        sqlx::query(&sql)
            .bind(q.state)
            .bind(q.since)
            .fetch_all(&st.db)
            .await
            .map_err(db_err)?
    } else {
        let sql = format!(
            "SELECT {ALERT_COLS} FROM alerts WHERE ($1::text IS NULL OR state = $1) \
             AND ($2::timestamptz IS NULL OR updated_at > $2) ORDER BY updated_at DESC LIMIT $3"
        );
        sqlx::query(&sql)
            .bind(q.state)
            .bind(q.since)
            .bind(limit)
            .fetch_all(&st.db)
            .await
            .map_err(db_err)?
    };

    let mut out = Vec::with_capacity(rows.len().min(limit as usize));
    for r in &rows {
        let (m, enriched) = summary(r, &rules);
        if let Some(team) = team_filter {
            if !enriched.teams.iter().any(|t| t == team) {
                continue;
            }
        }
        out.push(Value::Object(m));
        if out.len() >= limit as usize {
            break;
        }
    }
    Ok(Json(Value::Array(out)))
}

async fn fetch_alert(st: &AppState, id: Uuid) -> ApiResult<sqlx::postgres::PgRow> {
    sqlx::query(&format!("SELECT {ALERT_COLS} FROM alerts WHERE id = $1"))
        .bind(id)
        .fetch_optional(&st.db)
        .await
        .map_err(db_err)?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "alert not found", id))
}

async fn get_alert(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let r = fetch_alert(&st, parse_id(&id)?).await?;
    let rules = load_enabled_rules(&st.db).await;
    let (mut m, enriched) = summary(&r, &rules);
    m.insert("body".into(), json!(enriched.body));
    m.insert("fields".into(), Value::Array(enriched.fields));
    m.insert("event".into(), opt_json(&r, "last_event"));
    m.insert("acked_by".into(), opt_json(&r, "acked_by"));
    m.insert("resolved_by".into(), opt_json(&r, "resolved_by"));
    Ok(Json(Value::Object(m)))
}

async fn episodes(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let id = parse_id(&id)?;
    let rows = sqlx::query(
        "SELECT episode::int AS episode, opened_at, acked_at, resolved_at, count::bigint AS count, resolved_by FROM alert_episodes \
         WHERE alert_id = $1 ORDER BY episode DESC",
    )
    .bind(id)
    .fetch_all(&st.db)
    .await
    .map_err(db_err)?;
    let out: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "episode": r.get::<i32, _>("episode"),
                "opened_at": r.get::<DateTime<Utc>, _>("opened_at"),
                "acked_at": r.get::<Option<DateTime<Utc>>, _>("acked_at"),
                "resolved_at": r.get::<Option<DateTime<Utc>>, _>("resolved_at"),
                "count": r.get::<i64, _>("count"),
                "resolved_by": opt_json(r, "resolved_by"),
            })
        })
        .collect();
    Ok(Json(json!(out)))
}

/// Publikuje komendę na `alerts.commands`; stan zmienia wyłącznie lifecycle (single writer).
async fn command(st: AppState, id: String, headers: HeaderMap, kind: AlertCommandKind) -> ApiResult<StatusCode> {
    let id = parse_id(&id)?;
    let person_id = headers
        .get("x-person-id")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "missing X-Person-Id header", "wymagany nagłówek X-Person-Id"))?
        .to_string();
    let fingerprint: String = fetch_alert(&st, id).await?.get("fingerprint");
    let cmd = AlertCommand {
        alert_id: id,
        fingerprint: fingerprint.clone(),
        kind,
        actor: Actor::User { person_id },
        at: Utc::now(),
    };
    common::kafka::publish(&st.kafka, topics::ALERTS_COMMANDS, &fingerprint, &cmd)
        .await
        .map_err(|e| err(StatusCode::SERVICE_UNAVAILABLE, "kafka unavailable", e))?;
    Ok(StatusCode::ACCEPTED)
}

async fn ack(State(st): State<AppState>, Path(id): Path<String>, h: HeaderMap) -> ApiResult<StatusCode> {
    command(st, id, h, AlertCommandKind::Ack).await
}

async fn resolve(State(st): State<AppState>, Path(id): Path<String>, h: HeaderMap) -> ApiResult<StatusCode> {
    command(st, id, h, AlertCommandKind::Resolve).await
}

fn mapping_from_row(r: &sqlx::postgres::PgRow) -> ApiResult<MappingDef> {
    let mut def: MappingDef = serde_json::from_value(opt_json(r, "def"))
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, "invalid stored mapping", e))?;
    def.version = r.get::<i64, _>("version");
    Ok(def)
}

async fn list_mappings(State(st): State<AppState>) -> ApiResult<Json<Vec<MappingDef>>> {
    let rows = sqlx::query("SELECT def, version::bigint AS version FROM mappings ORDER BY schema")
        .fetch_all(&st.db)
        .await
        .map_err(db_err)?;
    rows.iter().map(mapping_from_row).collect::<ApiResult<Vec<_>>>().map(Json)
}

async fn get_mapping(State(st): State<AppState>, Path(schema): Path<String>) -> ApiResult<Json<MappingDef>> {
    let r = sqlx::query("SELECT def, version::bigint AS version FROM mappings WHERE schema = $1")
        .bind(&schema)
        .fetch_optional(&st.db)
        .await
        .map_err(db_err)?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "mapping not found", &schema))?;
    mapping_from_row(&r).map(Json)
}

async fn put_mapping(
    State(st): State<AppState>,
    Path(schema): Path<String>,
    body: Result<Json<MappingDef>, JsonRejection>,
) -> ApiResult<Json<MappingDef>> {
    let Json(mut def) = body.map_err(|e| err(StatusCode::BAD_REQUEST, "invalid mapping", e.body_text()))?;
    if def.schema != schema {
        return Err(err(StatusCode::BAD_REQUEST, "schema mismatch", "schema w body różni się od ścieżki"));
    }
    // Ta sama kompilacja co w lifecycle: mapping, który się nie kompiluje, nie trafia do bazy.
    CompiledMapping::compile(def.clone(), 0).map_err(|e| err(StatusCode::BAD_REQUEST, "invalid mapping", format!("{e:#}")))?;
    let stored = serde_json::to_value(&def).map_err(|e| err(StatusCode::BAD_REQUEST, "invalid mapping", e))?;
    let mut tx = st.db.begin().await.map_err(db_err)?;
    let version: i64 = sqlx::query(
        "INSERT INTO mappings (schema, def, version, updated_at) VALUES ($1, $2, 1, now()) \
         ON CONFLICT (schema) DO UPDATE SET def = EXCLUDED.def, version = mappings.version + 1, updated_at = now() \
         RETURNING version::bigint AS version",
    )
    .bind(&schema)
    .bind(SqlJson(stored))
    .fetch_one(&mut *tx)
    .await
    .map_err(db_err)?
    .get("version");
    sqlx::query("SELECT pg_notify('config_changed', 'mappings')").execute(&mut *tx).await.map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    def.version = version;
    Ok(Json(def))
}

async fn delete_mapping(State(st): State<AppState>, Path(schema): Path<String>) -> ApiResult<StatusCode> {
    let mut tx = st.db.begin().await.map_err(db_err)?;
    sqlx::query("DELETE FROM mappings WHERE schema = $1").bind(&schema).execute(&mut *tx).await.map_err(db_err)?;
    sqlx::query("SELECT pg_notify('config_changed', 'mappings')").execute(&mut *tx).await.map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct PreviewReq {
    mapping: MappingDef,
    payload: Value,
}

/// Podgląd zdarzenia kanonicznego: ten sam kod co normalizer w lifecycle, bez zapisu.
/// Payload-string traktujemy jako YAML (przykład wklejony w UI), resztę jako JSON.
async fn preview(body: Result<Json<PreviewReq>, JsonRejection>) -> ApiResult<Json<CanonicalEvent>> {
    let bad = |e: String| err(StatusCode::BAD_REQUEST, "invalid preview", e);
    let Json(req) = body.map_err(|e| bad(e.body_text()))?;
    let format = if req.payload.is_string() { formats::YAML } else { formats::JSON };
    let raw = parse_payload(format, &req.payload).map_err(|e| bad(format!("{e:#}")))?;
    let version = req.mapping.version;
    let mapping = CompiledMapping::compile(req.mapping, version).map_err(|e| bad(format!("{e:#}")))?;
    let env = Envelope {
        event_id: Uuid::now_v7(),
        format: format.into(),
        schema: mapping.def.schema.clone(),
        payload: req.payload,
        source: "preview".into(),
        received_at: Utc::now(),
    };
    Ok(Json(mapping.normalize(&env, raw)))
}

async fn unmapped(State(st): State<AppState>) -> ApiResult<Json<Value>> {
    let rows = sqlx::query(
        "SELECT g.schema, g.count, g.first_seen, g.last_seen, \
                (SELECT envelope->'payload' FROM unmapped_events u WHERE u.schema = g.schema \
                 ORDER BY received_at DESC LIMIT 1) AS sample \
         FROM (SELECT schema, count(*) AS count, min(received_at) AS first_seen, max(received_at) AS last_seen \
               FROM unmapped_events GROUP BY schema) g ORDER BY g.last_seen DESC",
    )
    .fetch_all(&st.db)
    .await
    .map_err(db_err)?;
    let out: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "schema": r.get::<String, _>("schema"),
                "count": r.get::<i64, _>("count"),
                "first_seen": r.get::<DateTime<Utc>, _>("first_seen"),
                "last_seen": r.get::<DateTime<Utc>, _>("last_seen"),
                "sample": opt_json(r, "sample"),
            })
        })
        .collect();
    Ok(Json(json!(out)))
}

#[cfg(test)]
mod tests {
    //! Testy integracyjne: wymagają działających Postgresa i Redpandy (docker compose).
    //! Router uruchamiany na losowym porcie, zapytania jako surowe HTTP/1.1.
    use std::time::Duration;

    use common::kafka::rdkafka::message::Message;
    use serde_json::{json, Value};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    use super::*;

    struct Server {
        addr: std::net::SocketAddr,
        st: AppState,
        alerts: std::sync::Mutex<Vec<Uuid>>,
    }

    async fn server() -> Server {
        let st = AppState::connect(common::Config::from_env()).await.expect("connect");
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = crate::router(st.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Server { addr, st, alerts: Default::default() }
    }

    impl Server {
        /// Domyślnie z `Authorization: Bearer <token z konfiguracji>`.
        async fn http(&self, method: &str, path: &str, headers: &[(&str, &str)], body: Option<&str>) -> (u16, Value) {
            let auth = format!("Bearer {}", self.st.config.api_token);
            let mut all = vec![("Authorization", auth.as_str())];
            all.extend_from_slice(headers);
            let (status, _, json) = self.http_raw(method, path, &all, body).await;
            (status, json)
        }

        /// Bez dodawania nagłówków; zwraca też nagłówki odpowiedzi.
        async fn http_raw(&self, method: &str, path: &str, headers: &[(&str, &str)], body: Option<&str>) -> (u16, String, Value) {
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

        /// Tylko status i nagłówki (dla strumieni SSE, które się nie kończą).
        async fn head_only(&self, path: &str) -> (u16, String) {
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

        /// Usuwa alerty wstawione przez `insert_alert` (wołać na końcu testu).
        async fn cleanup(&self) {
            let ids = self.alerts.lock().unwrap().clone();
            sqlx::query("DELETE FROM alert_episodes WHERE alert_id = ANY($1)").bind(&ids).execute(&self.st.db).await.unwrap();
            sqlx::query("DELETE FROM alerts WHERE id = ANY($1)").bind(&ids).execute(&self.st.db).await.unwrap();
        }

        async fn insert_alert(&self, fp: &str, state: &str, age_days: i32) -> Uuid {
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

    fn uniq(prefix: &str) -> String {
        format!("{prefix}-{}", Uuid::new_v4())
    }

    #[tokio::test]
    async fn list_alerts_filters() {
        let s = server().await;
        let a = s.insert_alert(&uniq("fp"), "acked", 0).await;
        let b = s.insert_alert(&uniq("fp"), "open", 0).await;
        let old = s.insert_alert(&uniq("fp"), "acked", 3).await;
        let ids = |v: &Value| -> Vec<String> { v.as_array().unwrap().iter().map(|x| x["id"].as_str().unwrap().to_string()).collect() };

        let (st, all) = s.http("GET", "/alerts?limit=1000", &[], None).await;
        assert_eq!(st, 200);
        assert!(ids(&all).contains(&a.to_string()));

        let (_, acked) = s.http("GET", "/alerts?state=acked&limit=1000", &[], None).await;
        assert!(ids(&acked).contains(&a.to_string()));
        assert!(!ids(&acked).contains(&b.to_string()));
        assert!(acked.as_array().unwrap().iter().all(|x| x["state"] == "acked"));

        let since: String = sqlx::query_scalar(
            "SELECT to_char((now() - interval '1 day') AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')",
        )
        .fetch_one(&s.st.db)
        .await
        .unwrap();
        let (_, recent) = s.http("GET", &format!("/alerts?since={since}&limit=1000"), &[], None).await;
        assert!(ids(&recent).contains(&a.to_string()));
        assert!(!ids(&recent).contains(&old.to_string()));

        let (_, one) = s.http("GET", "/alerts?limit=1", &[], None).await;
        assert_eq!(one.as_array().unwrap().len(), 1);
        s.cleanup().await;
    }

    #[tokio::test]
    async fn get_alert_details_and_errors() {
        let s = server().await;
        let id = s.insert_alert(&uniq("fp"), "acked", 0).await;
        let (st, v) = s.http("GET", &format!("/alerts/{id}"), &[], None).await;
        assert_eq!(st, 200);
        assert_eq!(v["id"], id.to_string());
        assert_eq!(v["event"]["title"], "tytul");
        assert_eq!(v["acked_by"], json!({"type": "user", "person_id": "ala"}));
        assert_eq!(v["episode"], 2);
        assert_eq!(v["count"], 5);

        let (st, _) = s.http("GET", &format!("/alerts/{}", Uuid::new_v4()), &[], None).await;
        assert_eq!(st, 404);
        let (st, _) = s.http("GET", "/alerts/nie-uuid", &[], None).await;
        assert_eq!(st, 400);
        s.cleanup().await;
    }

    #[tokio::test]
    async fn episodes_newest_first() {
        let s = server().await;
        let id = s.insert_alert(&uniq("fp"), "reopened", 0).await;
        for ep in [1, 2] {
            sqlx::query("INSERT INTO alert_episodes (alert_id, episode, opened_at, count) VALUES ($1, $2, now(), $3)")
                .bind(id)
                .bind(ep)
                .bind(ep as i64 * 10)
                .execute(&s.st.db)
                .await
                .unwrap();
        }
        let (st, v) = s.http("GET", &format!("/alerts/{id}/episodes"), &[], None).await;
        assert_eq!(st, 200);
        let eps: Vec<i64> = v.as_array().unwrap().iter().map(|e| e["episode"].as_i64().unwrap()).collect();
        assert_eq!(eps, vec![2, 1]);
        assert_eq!(v[0]["count"], 20);
        s.cleanup().await;
    }

    async fn find_command(alert_id: Uuid) -> AlertCommand {
        let cfg = common::Config::from_env();
        let consumer = common::kafka::consumer(&cfg.kafka_brokers, &uniq("p1-test"), &[topics::ALERTS_COMMANDS]).unwrap();
        let fut = async {
            loop {
                let m = consumer.recv().await.unwrap();
                let key = String::from_utf8(m.key().unwrap().to_vec()).unwrap();
                if let Ok(cmd) = common::kafka::decode::<AlertCommand>(&m) {
                    if cmd.alert_id == alert_id {
                        assert_eq!(key, cmd.fingerprint);
                        return cmd;
                    }
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(15), fut).await.expect("brak komendy na alerts.commands")
    }

    #[tokio::test]
    async fn ack_publishes_command() {
        let s = server().await;
        let fp = uniq("fp");
        let id = s.insert_alert(&fp, "open", 0).await;

        let (st, _) = s.http("POST", &format!("/alerts/{id}/ack"), &[], None).await;
        assert_eq!(st, 400);
        let (st, _) = s.http("POST", &format!("/alerts/{}/ack", Uuid::new_v4()), &[("X-Person-Id", "ala")], None).await;
        assert_eq!(st, 404);

        let (st, _) = s.http("POST", &format!("/alerts/{id}/ack"), &[("X-Person-Id", "ala")], None).await;
        assert_eq!(st, 202);
        let cmd = find_command(id).await;
        assert_eq!(cmd.fingerprint, fp);
        assert!(matches!(cmd.kind, AlertCommandKind::Ack));
        assert_eq!(cmd.actor, Actor::User { person_id: "ala".into() });
        s.cleanup().await;
    }

    #[tokio::test]
    async fn resolve_accepted() {
        let s = server().await;
        let id = s.insert_alert(&uniq("fp"), "open", 0).await;
        let (st, _) = s.http("POST", &format!("/alerts/{id}/resolve"), &[("X-Person-Id", "bob")], None).await;
        assert_eq!(st, 202);
        let (st, _) = s.http("POST", &format!("/alerts/{id}/resolve"), &[], None).await;
        assert_eq!(st, 400);
        s.cleanup().await;
    }

    #[tokio::test]
    async fn mappings_crud_and_notify() {
        let s = server().await;
        let schema = uniq("t-schema");
        let body = json!({"schema": schema, "severity": {"type": "path", "path": "$.s", "map": {}, "default": "warning"}}).to_string();
        let path = format!("/mappings/{schema}");

        let mut listener = sqlx::postgres::PgListener::connect_with(&s.st.db).await.unwrap();
        listener.listen("config_changed").await.unwrap();

        let (st, v1) = s.http("PUT", &path, &[], Some(&body)).await;
        assert_eq!(st, 200);
        let (st, v2) = s.http("PUT", &path, &[], Some(&body)).await;
        assert_eq!(st, 200);
        assert_eq!(v2["version"].as_i64().unwrap(), v1["version"].as_i64().unwrap() + 1);

        let n = tokio::time::timeout(Duration::from_secs(5), listener.recv()).await.expect("brak NOTIFY").unwrap();
        assert_eq!(n.channel(), "config_changed");
        assert_eq!(n.payload(), "mappings");

        let (st, v) = s.http("GET", &path, &[], None).await;
        assert_eq!(st, 200);
        assert_eq!(v["schema"], schema);
        let (_, list) = s.http("GET", "/mappings", &[], None).await;
        assert!(list.as_array().unwrap().iter().any(|m| m["schema"] == schema.as_str()));

        let (st, _) = s.http("PUT", &path, &[], Some(&body.replace(&schema, "inny"))).await;
        assert_eq!(st, 400);
        let (st, _) = s.http("PUT", &path, &[], Some("{nie json")).await;
        assert_eq!(st, 400);

        let (st, _) = s.http("DELETE", &path, &[], None).await;
        assert_eq!(st, 204);
        let (st, _) = s.http("GET", &path, &[], None).await;
        assert_eq!(st, 404);
        let (st, _) = s.http("GET", "/mappings/nie-istnieje-xyz", &[], None).await;
        assert_eq!(st, 404);
    }

    #[tokio::test]
    async fn unmapped_aggregates() {
        let s = server().await;
        let schema = uniq("unm");
        for i in 0..3 {
            sqlx::query(
                "INSERT INTO unmapped_events (event_id, schema, envelope, received_at) \
                 VALUES ($1, $2, $3, now() - make_interval(mins => $4))",
            )
            .bind(Uuid::new_v4())
            .bind(&schema)
            .bind(SqlJson(json!({"payload": {"n": i}})))
            .bind(10 - i)
            .execute(&s.st.db)
            .await
            .unwrap();
        }
        let (st, v) = s.http("GET", "/unmapped", &[], None).await;
        assert_eq!(st, 200);
        let row = v.as_array().unwrap().iter().find(|r| r["schema"] == schema.as_str()).expect("brak schematu");
        assert_eq!(row["count"], 3);
        assert_eq!(row["sample"], json!({"n": 2}));
        sqlx::query("DELETE FROM unmapped_events WHERE schema = $1").bind(&schema).execute(&s.st.db).await.unwrap();
    }

    #[tokio::test]
    async fn ingest_errors_listed() {
        let s = server().await;
        let src = uniq("src");
        sqlx::query("INSERT INTO ingest_errors (source, error, sample) VALUES ($1, 'zly json', '{')")
            .bind(&src)
            .execute(&s.st.db)
            .await
            .unwrap();
        let (st, v) = s.http("GET", "/ingest/errors", &[], None).await;
        assert_eq!(st, 200);
        let e = v.as_array().unwrap().iter().find(|e| e["source"] == src.as_str()).expect("brak błędu");
        assert_eq!(e["error"], "zly json");
        assert_eq!(e["sample"], "{");
        sqlx::query("DELETE FROM ingest_errors WHERE source = $1").bind(&src).execute(&s.st.db).await.unwrap();
    }

    const WORKFLOW_MAPPING: &str = include_str!("../../contracts/fixtures/config/mapping.workflow.json");
    const WORKFLOW_EVENT: &str = include_str!("../../contracts/fixtures/events/workflow_failed.json");

    async fn mapping_rows(s: &Server, schema: &str) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM mappings WHERE schema = $1").bind(schema).fetch_one(&s.st.db).await.unwrap()
    }

    #[tokio::test]
    async fn preview_workflow_json_payload() {
        let s = server().await;
        let mapping: Value = serde_json::from_str(WORKFLOW_MAPPING).unwrap();
        let payload = serde_json::from_str::<Value>(WORKFLOW_EVENT).unwrap()["payload"].clone();
        let body = json!({"mapping": mapping, "payload": payload}).to_string();
        let (st, v) = s.http("POST", "/mappings/preview", &[], Some(&body)).await;
        assert_eq!(st, 200, "{v}");
        assert_eq!(v["severity"], "error");
        assert_eq!(v["title"], "Counterparty service did not respond in 5000 ms");
        assert_eq!(v["fingerprint"], "workflow.v1|order-settlement|validate-counterparty|TIMEOUT");
        assert_eq!(v["raw"], payload);
        assert_eq!(v["fields"]["step"], "validate-counterparty");
    }

    #[tokio::test]
    async fn preview_yaml_string_payload() {
        let s = server().await;
        let mapping: Value = serde_json::from_str(WORKFLOW_MAPPING).unwrap();
        let yaml = "workflowName: order-settlement\nstep: validate-counterparty\nseverity: WARN\nerror:\n  code: TIMEOUT\n  message: boom\n";
        let body = json!({"mapping": mapping, "payload": yaml}).to_string();
        let (st, v) = s.http("POST", "/mappings/preview", &[], Some(&body)).await;
        assert_eq!(st, 200, "{v}");
        assert_eq!(v["fields"]["workflowName"], "order-settlement");
        assert_eq!(v["fields"]["step"], "validate-counterparty");
        assert_eq!(v["severity"], "warning");
        assert_eq!(v["title"], "boom");
    }

    #[tokio::test]
    async fn preview_errors_are_400_and_nothing_is_stored() {
        let s = server().await;
        let schema = uniq("prev");
        let mut mapping: Value = serde_json::from_str(WORKFLOW_MAPPING).unwrap();
        mapping["schema"] = json!(schema);

        let mut bad = mapping.clone();
        bad["fields"]["x"] = json!("$[[[");
        let body = json!({"mapping": bad, "payload": {"a": 1}}).to_string();
        let (st, _) = s.http("POST", "/mappings/preview", &[], Some(&body)).await;
        assert_eq!(st, 400);

        let body = json!({"mapping": mapping}).to_string();
        let (st, _) = s.http("POST", "/mappings/preview", &[], Some(&body)).await;
        assert_eq!(st, 400);

        let body = json!({"mapping": mapping, "payload": "a: [unclosed"}).to_string();
        let (st, _) = s.http("POST", "/mappings/preview", &[], Some(&body)).await;
        assert_eq!(st, 400);

        let body = json!({"mapping": mapping, "payload": {"workflowName": "w"}}).to_string();
        let (st, _) = s.http("POST", "/mappings/preview", &[], Some(&body)).await;
        assert_eq!(st, 200);
        assert_eq!(mapping_rows(&s, &schema).await, 0);
    }

    #[tokio::test]
    async fn put_rejects_uncompilable_mapping() {
        let s = server().await;
        let schema = uniq("badput");
        let mut mapping: Value = serde_json::from_str(WORKFLOW_MAPPING).unwrap();
        mapping["schema"] = json!(schema);
        mapping["fields"]["x"] = json!("$[[[");
        let (st, _) = s.http("PUT", &format!("/mappings/{schema}"), &[], Some(&mapping.to_string())).await;
        assert_eq!(st, 400);
        assert_eq!(mapping_rows(&s, &schema).await, 0);
    }

    #[tokio::test]
    async fn auth_rules() {
        let s = server().await;
        let tok = s.st.config.api_token.clone();
        let (st, _, _) = s.http_raw("GET", "/health", &[], None).await;
        assert_eq!(st, 200);

        let (st, _, v) = s.http_raw("GET", "/alerts", &[], None).await;
        assert_eq!((st, &v), (401, &json!({"error": "unauthorized"})));
        let (st, _, _) = s.http_raw("GET", "/alerts", &[("Authorization", "Bearer zly")], None).await;
        assert_eq!(st, 401);
        let (st, _, _) = s.http_raw("GET", "/alerts?limit=1", &[("Authorization", &format!("Bearer {tok}"))], None).await;
        assert_eq!(st, 200);

        let (st, _, _) = s.http_raw("GET", &format!("/alerts?limit=1&token={tok}"), &[], None).await;
        assert_eq!(st, 200);
        let (st, _, _) = s.http_raw("GET", "/alerts?token=zly", &[], None).await;
        assert_eq!(st, 401);

        let (st, _) = s.head_only("/stream").await;
        assert_eq!(st, 401);
        let (st, head) = s.head_only(&format!("/stream?token={tok}")).await;
        assert_eq!(st, 200);
        assert!(head.contains("content-type: text/event-stream"), "{head}");

        let (st, head, _) = s
            .http_raw("OPTIONS", "/alerts", &[("Origin", "http://localhost:5173"), ("Access-Control-Request-Method", "GET")], None)
            .await;
        assert_ne!(st, 401);
        assert!(head.contains("access-control-allow-origin"), "{head}");
    }

    #[test]
    fn unit_enrich_alert_settlements_presentation_and_override() {
        let rule: RuleDef = serde_json::from_str(include_str!("../../contracts/fixtures/config/rule.settlements.json")).unwrap();
        let compiled = CompiledRule::from_def(rule).unwrap();
        let alert: contracts::AlertChanged =
            serde_json::from_str(include_str!("../../contracts/fixtures/topics/alerts.changed.json")).unwrap();
        let last_event = serde_json::to_value(&alert.event).unwrap();

        let e = enrich_alert("error", &alert.event.title, &last_event, &[compiled]);
        assert_eq!(e.rule_ids, vec!["settlements-prod"]);
        assert_eq!(e.teams, vec!["settlements"]);
        assert_eq!(e.severity, "critical");
        assert_eq!(e.title, "WORKFLOW_FAILED order-settlement");
        assert_eq!(e.color.as_deref(), Some("red"));
        assert_eq!(e.body, "Step validate-counterparty failed: Counterparty service did not respond in 5000 ms");
        assert_eq!(e.fields.len(), 3);
        assert_eq!(e.fields[0]["path"], "raw.workflowId");
        assert_eq!(e.fields[0]["label"], "workflowId");
        assert_eq!(e.fields[0]["value"], "WF-20261023-00042");
        assert_eq!(e.fields[0]["highlight"], Value::Null);
        assert_eq!(e.fields[1]["path"], "raw.step");
        assert_eq!(e.fields[1]["highlight"], "red");
        assert_eq!(e.fields[2]["path"], "raw.error.message");
        assert_eq!(e.fields[2]["highlight"], "red");
    }

    #[test]
    fn unit_enrich_alert_unmatched_and_non_canonical_fallback() {
        let rule: RuleDef = serde_json::from_str(include_str!("../../contracts/fixtures/config/rule.rates.json")).unwrap();
        let compiled = CompiledRule::from_def(rule).unwrap();
        let alert: contracts::AlertChanged =
            serde_json::from_str(include_str!("../../contracts/fixtures/topics/alerts.changed.json")).unwrap();
        let last_event = serde_json::to_value(&alert.event).unwrap();

        let e = enrich_alert("error", "base-title", &last_event, std::slice::from_ref(&compiled));
        assert!(e.rule_ids.is_empty());
        assert!(e.teams.is_empty());
        assert_eq!(e.severity, "error");
        assert_eq!(e.title, "base-title");
        assert!(e.color.is_none());

        // Niepełny JSON (jak w starszych wierszach testowych) nie psuje odpowiedzi.
        let e2 = enrich_alert("warning", "tytul", &json!({"title": "tytul"}), &[compiled]);
        assert!(e2.rule_ids.is_empty());
        assert_eq!(e2.severity, "warning");
        assert_eq!(e2.title, "tytul");
    }

    #[tokio::test]
    async fn list_alerts_team_filter_and_presentation() {
        let s = server().await;
        let rule_id = uniq("d10-rule");
        let team_id = uniq("d10-team");
        let fp_match = uniq("d10-fp-match");
        let fp_other = uniq("d10-fp-other");

        let mut alert_changed: contracts::AlertChanged =
            serde_json::from_str(include_str!("../../contracts/fixtures/topics/alerts.changed.json")).unwrap();
        alert_changed.event.fingerprint = fp_match.clone();
        let ev_match = serde_json::to_value(&alert_changed.event).unwrap();

        let rule: RuleDef = serde_json::from_value(json!({
            "id": rule_id,
            "name": "D10 test rule",
            "team": team_id,
            "condition": { "field": "fingerprint", "op": "eq", "value": fp_match },
            "recipients": [{ "type": "team", "id": team_id }],
            "severity_override": "critical",
            "presentation": {
                "title": "D10 {{raw.workflowName}}",
                "body": "Step {{raw.step}}",
                "fields": ["raw.step"],
                "highlight": { "raw.step": "red" },
                "color": "red"
            },
            "enabled": true,
            "version": 1
        }))
        .unwrap();

        sqlx::query("INSERT INTO rules (id, name, team, def, enabled, version, updated_at) VALUES ($1, $2, $3, $4, TRUE, 1, now())")
            .bind(&rule.id)
            .bind(&rule.name)
            .bind(&rule.team)
            .bind(SqlJson(serde_json::to_value(&rule).unwrap()))
            .execute(&s.st.db)
            .await
            .unwrap();

        let id_match = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO alerts (id, fingerprint, state, severity, title, schema, episode, count, first_seen, last_seen, last_event, updated_at) \
             VALUES ($1, $2, 'open', 'error', 'orig', 'workflow.v1', 1, 1, now(), now(), $3, now())",
        )
        .bind(id_match)
        .bind(&fp_match)
        .bind(SqlJson(ev_match))
        .execute(&s.st.db)
        .await
        .unwrap();
        s.alerts.lock().unwrap().push(id_match);
        let id_other = s.insert_alert(&fp_other, "open", 0).await;

        let (st, list) = s.http("GET", &format!("/alerts?team={team_id}"), &[], None).await;
        assert_eq!(st, 200);
        let arr = list.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"], id_match.to_string());
        assert_eq!(arr[0]["severity"], "critical");
        assert_eq!(arr[0]["title"], "D10 order-settlement");
        assert_eq!(arr[0]["color"], "red");
        assert_eq!(arr[0]["rule_ids"], json!([rule_id]));
        assert_eq!(arr[0]["teams"], json!([team_id]));
        assert_ne!(arr[0]["id"], id_other.to_string());

        let (st, detail) = s.http("GET", &format!("/alerts/{id_match}"), &[], None).await;
        assert_eq!(st, 200);
        assert_eq!(detail["body"], "Step validate-counterparty");
        assert_eq!(detail["fields"][0]["path"], "raw.step");
        assert_eq!(detail["fields"][0]["highlight"], "red");

        sqlx::query("DELETE FROM rules WHERE id = $1").bind(&rule.id).execute(&s.st.db).await.unwrap();
        s.cleanup().await;
    }
}
