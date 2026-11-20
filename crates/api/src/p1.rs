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
use contracts::{topics, Actor, AlertCommand, AlertCommandKind, MappingDef};
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

/// shortcut: rule_ids/teams/color/prezentacja wymagają reguł P2 (tabela rules + condition); uzupełnić przy integracji 18.x
fn summary(r: &sqlx::postgres::PgRow) -> serde_json::Map<String, Value> {
    let v = json!({
        "id": r.get::<Uuid, _>("id"),
        "fingerprint": r.get::<String, _>("fingerprint"),
        "state": r.get::<String, _>("state"),
        "severity": r.get::<String, _>("severity"),
        "title": r.get::<String, _>("title"),
        "schema": r.get::<String, _>("schema"),
        "count": r.get::<i64, _>("count"),
        "episode": r.get::<i32, _>("episode"),
        "first_seen": r.get::<DateTime<Utc>, _>("first_seen"),
        "last_seen": r.get::<DateTime<Utc>, _>("last_seen"),
        "rule_ids": [],
        "teams": [],
        "color": null,
    });
    match v {
        Value::Object(m) => m,
        _ => unreachable!(),
    }
}

#[derive(Deserialize)]
struct AlertsQuery {
    state: Option<String>,
    since: Option<DateTime<Utc>>,
    limit: Option<i64>,
    /// Akceptowany, ale ignorowany (patrz `summary`).
    #[allow(dead_code)]
    team: Option<String>,
}

async fn list_alerts(State(st): State<AppState>, Query(q): Query<AlertsQuery>) -> ApiResult<Json<Value>> {
    let limit = q.limit.unwrap_or(100).clamp(1, 1000);
    let sql = format!(
        "SELECT {ALERT_COLS} FROM alerts WHERE ($1::text IS NULL OR state = $1) \
         AND ($2::timestamptz IS NULL OR updated_at > $2) ORDER BY updated_at DESC LIMIT $3"
    );
    let rows = sqlx::query(&sql)
        .bind(q.state)
        .bind(q.since)
        .bind(limit)
        .fetch_all(&st.db)
        .await
        .map_err(db_err)?;
    Ok(Json(Value::Array(rows.iter().map(|r| Value::Object(summary(r))).collect())))
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
    let mut m = summary(&r);
    m.insert("body".into(), json!(""));
    m.insert("fields".into(), json!([]));
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

async fn preview() -> (StatusCode, Json<Value>) {
    err(
        StatusCode::NOT_IMPLEMENTED,
        "not implemented",
        "preview wymaga zależności api -> lifecycle (zmiana crates/api/Cargo.toml do uzgodnienia)",
    )
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
