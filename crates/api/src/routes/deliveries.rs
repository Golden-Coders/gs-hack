//! Deliveries and simulated channels routes.

use std::collections::BTreeMap;

use axum::{
    extract::{rejection::JsonRejection, Path, Query, State},
    http::StatusCode,
    routing::{get, post, put},
    Json, Router,
};
use common::db::sqlx::{
    self,
    types::{chrono::DateTime, chrono::Utc, Json as SqlJson, Uuid},
    Row,
};
use contracts::{topics, Channel, DeliveryRequest, Severity};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{db_err, err, opt_json, parse_uuid, ApiResult};
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/alerts/{alert_id}/deliveries", get(alert_deliveries))
        .route("/deliveries", get(list_deliveries))
        .route("/deliveries/{delivery_id}/resend", post(resend_delivery))
        .route("/mock/outbox", get(mock_outbox))
        .route("/mock/channels", get(list_mock_channels))
        .route("/mock/channels/{channel}", put(put_mock_channel))
}

const DELIVERY_COLS: &str = "id, alert_id, episode::int AS episode, person_id, channel, status, \
                             attempts::int AS attempts, last_error, received_at, delivered_at, updated_at";

fn delivery_row_to_json(r: &sqlx::postgres::PgRow) -> Value {
    json!({
        "id": r.get::<Uuid, _>("id"),
        "alert_id": r.get::<Uuid, _>("alert_id"),
        "episode": r.get::<i32, _>("episode"),
        "person_id": r.get::<String, _>("person_id"),
        "channel": r.get::<String, _>("channel"),
        "status": r.get::<String, _>("status"),
        "attempts": r.get::<i32, _>("attempts"),
        "last_error": r.get::<Option<String>, _>("last_error"),
        "received_at": r.get::<DateTime<Utc>, _>("received_at"),
        "delivered_at": r.get::<Option<DateTime<Utc>>, _>("delivered_at"),
        "updated_at": r.get::<DateTime<Utc>, _>("updated_at"),
    })
}

async fn alert_deliveries(
    State(st): State<AppState>,
    Path(alert_id): Path<String>,
) -> ApiResult<Json<Vec<Value>>> {
    let alert_id = parse_uuid(&alert_id, "alert_id")?;
    let sql = format!(
        "SELECT {DELIVERY_COLS} FROM deliveries WHERE alert_id = $1 ORDER BY received_at ASC, updated_at ASC, id ASC"
    );
    let rows = sqlx::query(&sql)
        .bind(alert_id)
        .fetch_all(&st.db)
        .await
        .map_err(db_err)?;
    Ok(Json(rows.iter().map(delivery_row_to_json).collect()))
}

#[derive(Deserialize)]
struct DeliveriesQuery {
    status: Option<String>,
}

async fn list_deliveries(
    State(st): State<AppState>,
    Query(q): Query<DeliveriesQuery>,
) -> ApiResult<Json<Vec<Value>>> {
    let sql = format!(
        "SELECT {DELIVERY_COLS} FROM deliveries \
         WHERE ($1::text IS NULL OR status = $1) \
         ORDER BY updated_at DESC LIMIT 200"
    );
    let rows = sqlx::query(&sql)
        .bind(q.status)
        .fetch_all(&st.db)
        .await
        .map_err(db_err)?;
    Ok(Json(rows.iter().map(delivery_row_to_json).collect()))
}

async fn resend_delivery(
    State(st): State<AppState>,
    Path(delivery_id): Path<String>,
) -> ApiResult<StatusCode> {
    let delivery_id = parse_uuid(&delivery_id, "delivery_id")?;
    let row = sqlx::query(
        "SELECT id, alert_id, episode::int AS episode, person_id, channel, request, received_at \
         FROM deliveries WHERE id = $1",
    )
    .bind(delivery_id)
    .fetch_optional(&st.db)
    .await
    .map_err(db_err)?
    .ok_or_else(|| err(StatusCode::NOT_FOUND, "delivery not found", delivery_id))?;

    let channel_str: String = row.get("channel");
    let target_channel: Channel =
        serde_json::from_value(Value::String(channel_str.clone())).unwrap_or(Channel::Email);

    let mut req: DeliveryRequest = match serde_json::from_value(opt_json(&row, "request")) {
        Ok(r) => r,
        Err(_) => {
            let alert_id: Uuid = row.get("alert_id");
            let episode: i32 = row.get("episode");
            let person_id: String = row.get("person_id");
            let received_at: DateTime<Utc> = row.get("received_at");
            DeliveryRequest {
                delivery_id,
                alert_id,
                episode,
                person_id,
                severity: Severity::Critical,
                origin: contracts::DeliveryOrigin::Rule {
                    rule_id: "resend".into(),
                },
                chain: vec![target_channel, Channel::Ui],
                chain_index: 0,
                contacts: BTreeMap::new(),
                rendered: BTreeMap::new(),
                received_at,
                attempt: 0,
                not_before: None,
                last_error: None,
            }
        }
    };

    req.delivery_id = delivery_id;
    req.attempt = 0;
    req.not_before = None;
    req.last_error = None;
    if let Some(idx) = req.chain.iter().position(|c| *c == target_channel) {
        req.chain_index = idx;
    } else {
        req.chain_index = 0;
    }

    let req_json = serde_json::to_value(&req)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, "serialize delivery request", e))?;

    sqlx::query(
        "UPDATE deliveries SET status = 'pending', attempts = 0, last_error = NULL, request = $2, updated_at = now() WHERE id = $1",
    )
    .bind(delivery_id)
    .bind(SqlJson(req_json))
    .execute(&st.db)
    .await
    .map_err(db_err)?;

    let key = req.alert_id.to_string();
    common::kafka::publish(&st.kafka, topics::DELIVERIES_REQUESTED, &key, &req)
        .await
        .map_err(|e| err(StatusCode::SERVICE_UNAVAILABLE, "kafka unavailable", e))?;

    Ok(StatusCode::ACCEPTED)
}

async fn mock_outbox(State(st): State<AppState>) -> ApiResult<Json<Vec<Value>>> {
    let rows = sqlx::query(
        "SELECT id::bigint AS id, channel, recipient, person_id, title, body, alert_id, sent_at \
         FROM mock_outbox ORDER BY sent_at DESC, id DESC LIMIT 200",
    )
    .fetch_all(&st.db)
    .await
    .map_err(db_err)?;

    let out: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<i64, _>("id"),
                "channel": r.get::<String, _>("channel"),
                "to": r.get::<String, _>("recipient"),
                "person_id": r.get::<String, _>("person_id"),
                "title": r.get::<String, _>("title"),
                "body": r.get::<String, _>("body"),
                "alert_id": r.get::<Option<Uuid>, _>("alert_id"),
                "sent_at": r.get::<DateTime<Utc>, _>("sent_at"),
            })
        })
        .collect();
    Ok(Json(out))
}

async fn list_mock_channels(State(st): State<AppState>) -> ApiResult<Json<Vec<Value>>> {
    let rows = sqlx::query(
        "SELECT channel, available FROM mock_channels \
         WHERE channel IN ('sms', 'phone', 'email') \
         ORDER BY CASE channel WHEN 'sms' THEN 1 WHEN 'phone' THEN 2 WHEN 'email' THEN 3 ELSE 4 END",
    )
    .fetch_all(&st.db)
    .await
    .map_err(db_err)?;

    let out: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "channel": r.get::<String, _>("channel"),
                "available": r.get::<bool, _>("available"),
            })
        })
        .collect();
    Ok(Json(out))
}

#[derive(Deserialize)]
struct ToggleChannelRequest {
    available: bool,
}

async fn put_mock_channel(
    State(st): State<AppState>,
    Path(channel): Path<String>,
    body: Result<Json<ToggleChannelRequest>, JsonRejection>,
) -> ApiResult<Json<Value>> {
    let parsed_channel: Channel = serde_json::from_value(Value::String(channel.clone()))
        .map_err(|_| err(StatusCode::BAD_REQUEST, "invalid channel", &channel))?;
    let channel_str = match parsed_channel {
        Channel::Ui => "ui",
        Channel::Email => "email",
        Channel::Sms => "sms",
        Channel::Phone => "phone",
    };
    let Json(req) =
        body.map_err(|e| err(StatusCode::BAD_REQUEST, "invalid request body", e.body_text()))?;

    let mut tx = st.db.begin().await.map_err(db_err)?;
    sqlx::query(
        "INSERT INTO mock_channels (channel, available, updated_at) \
         VALUES ($1, $2, now()) \
         ON CONFLICT (channel) DO UPDATE SET available = EXCLUDED.available, updated_at = now()",
    )
    .bind(channel_str)
    .bind(req.available)
    .execute(&mut *tx)
    .await
    .map_err(db_err)?;

    sqlx::query("SELECT pg_notify('config_changed', 'mock_channels')")
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;

    Ok(Json(json!({
        "channel": channel_str,
        "available": req.available,
    })))
}
