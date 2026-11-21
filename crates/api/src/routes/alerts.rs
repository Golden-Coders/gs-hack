//! Alerts and ingest error routes.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use common::db::sqlx::{
    self,
    types::{chrono::DateTime, chrono::Utc, Uuid},
    Row,
};
use contracts::{topics, Actor, AlertCommand, AlertCommandKind};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{db_err, err, opt_json, parse_uuid, ApiResult};
use crate::rules_eval::{enrich_alert, load_enabled_rules, AlertEnrichment, CompiledRule};
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/ingest/errors", get(ingest_errors))
        .route("/alerts", get(list_alerts))
        .route("/alerts/{alert_id}", get(get_alert))
        .route("/alerts/{alert_id}/episodes", get(episodes))
        .route("/alerts/{alert_id}/ack", post(ack))
        .route("/alerts/{alert_id}/resolve", post(resolve))
}

async fn ingest_errors(State(st): State<AppState>) -> ApiResult<Json<Value>> {
    let rows = sqlx::query(
        "SELECT source, error, sample, at FROM ingest_errors ORDER BY at DESC LIMIT 100",
    )
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

fn summary(
    r: &sqlx::postgres::PgRow,
    rules: &[CompiledRule],
) -> (serde_json::Map<String, Value>, AlertEnrichment) {
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

async fn list_alerts(
    State(st): State<AppState>,
    Query(q): Query<AlertsQuery>,
) -> ApiResult<Json<Value>> {
    let limit = q.limit.unwrap_or(100).clamp(1, 1000);
    let team_filter = q.team.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let rules = load_enabled_rules(&st.db).await;

    let query_limit = if team_filter.is_some() {
        (limit * 20).clamp(2000, 10_000)
    } else {
        limit
    };
    let sql = format!(
        "SELECT {ALERT_COLS} FROM alerts WHERE ($1::text IS NULL OR state = $1) \
         AND ($2::timestamptz IS NULL OR updated_at > $2) ORDER BY updated_at DESC LIMIT $3"
    );
    let rows = sqlx::query(&sql)
        .bind(q.state)
        .bind(q.since)
        .bind(query_limit)
        .fetch_all(&st.db)
        .await
        .map_err(db_err)?;

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
    let r = fetch_alert(&st, parse_uuid(&id, "alert_id")?).await?;
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
    let id = parse_uuid(&id, "alert_id")?;
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

/// Publishes a command to `alerts.commands`; state is mutated solely by `lifecycle` (single writer).
async fn command(
    st: AppState,
    id: String,
    auth: crate::auth::AuthenticatedOperator,
    headers: HeaderMap,
    kind: AlertCommandKind,
) -> ApiResult<StatusCode> {
    let id = parse_uuid(&id, "alert_id")?;
    let person_id = crate::auth::resolve_verified_person_id(&st, &auth, &headers).await?;
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

async fn ack(
    State(st): State<AppState>,
    Path(id): Path<String>,
    axum::Extension(auth): axum::Extension<crate::auth::AuthenticatedOperator>,
    h: HeaderMap,
) -> ApiResult<StatusCode> {
    command(st, id, auth, h, AlertCommandKind::Ack).await
}

async fn resolve(
    State(st): State<AppState>,
    Path(id): Path<String>,
    axum::Extension(auth): axum::Extension<crate::auth::AuthenticatedOperator>,
    h: HeaderMap,
) -> ApiResult<StatusCode> {
    command(st, id, auth, h, AlertCommandKind::Resolve).await
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use common::db::sqlx::{
        self,
        types::{Json as SqlJson, Uuid},
    };
    use common::kafka::rdkafka::message::Message;
    use contracts::{topics, Actor, AlertCommand, AlertCommandKind, RuleDef};
    use serde_json::{json, Value};

    use crate::routes::test_util::{server, uniq};

    #[tokio::test]
    async fn list_alerts_filters() {
        let s = server().await;
        let a = s.insert_alert(&uniq("fp"), "acked", 0).await;
        let b = s.insert_alert(&uniq("fp"), "open", 0).await;
        let old = s.insert_alert(&uniq("fp"), "acked", 3).await;
        let ids = |v: &Value| -> Vec<String> {
            v.as_array()
                .unwrap()
                .iter()
                .map(|x| x["id"].as_str().unwrap().to_string())
                .collect()
        };

        let (st, all) = s.http("GET", "/alerts?limit=1000", &[], None).await;
        assert_eq!(st, 200);
        assert!(ids(&all).contains(&a.to_string()));

        let (_, acked) = s
            .http("GET", "/alerts?state=acked&limit=1000", &[], None)
            .await;
        assert!(ids(&acked).contains(&a.to_string()));
        assert!(!ids(&acked).contains(&b.to_string()));
        assert!(acked
            .as_array()
            .unwrap()
            .iter()
            .all(|x| x["state"] == "acked"));

        let since: String = sqlx::query_scalar(
            "SELECT to_char((now() - interval '1 day') AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')",
        )
        .fetch_one(&s.st.db)
        .await
        .unwrap();
        let (_, recent) = s
            .http(
                "GET",
                &format!("/alerts?since={since}&limit=1000"),
                &[],
                None,
            )
            .await;
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

        let (st, _) = s
            .http("GET", &format!("/alerts/{}", Uuid::new_v4()), &[], None)
            .await;
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
        let (st, v) = s
            .http("GET", &format!("/alerts/{id}/episodes"), &[], None)
            .await;
        assert_eq!(st, 200);
        let eps: Vec<i64> = v
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["episode"].as_i64().unwrap())
            .collect();
        assert_eq!(eps, vec![2, 1]);
        assert_eq!(v[0]["count"], 20);
        s.cleanup().await;
    }

    async fn find_command(alert_id: Uuid) -> AlertCommand {
        let cfg = common::Config::from_env();
        let consumer = common::kafka::consumer(
            &cfg.kafka_brokers,
            &uniq("alerts-test"),
            &[topics::ALERTS_COMMANDS],
        )
        .unwrap();
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
        tokio::time::timeout(Duration::from_secs(15), fut)
            .await
            .expect("brak komendy na alerts.commands")
    }

    #[tokio::test]
    async fn ack_publishes_command() {
        let s = server().await;
        let fp = uniq("fp");
        let id = s.insert_alert(&fp, "open", 0).await;
        sqlx::query("INSERT INTO people (id, name, contacts, def) VALUES ('ala', 'Ala', '{}'::jsonb, '{}'::jsonb) ON CONFLICT (id) DO NOTHING")
            .execute(&s.st.db)
            .await
            .unwrap();

        let (st, _) = s
            .http(
                "POST",
                &format!("/alerts/{id}/ack"),
                &[("X-Person-Id", "ala")],
                None,
            )
            .await;
        assert_eq!(st, 403);

        let sess = s.st.auth.create_session("ala".into());
        let auth_hdr = format!("Bearer {sess}");

        let (st, _, _) = s
            .http_raw(
                "POST",
                &format!("/alerts/{id}/ack"),
                &[("Authorization", &auth_hdr), ("X-Person-Id", "bob")],
                None,
            )
            .await;
        assert_eq!(st, 403);

        let (st, _, _) = s
            .http_raw(
                "POST",
                &format!("/alerts/{}/ack", Uuid::new_v4()),
                &[("Authorization", &auth_hdr)],
                None,
            )
            .await;
        assert_eq!(st, 404);

        let (st, _, _) = s
            .http_raw(
                "POST",
                &format!("/alerts/{id}/ack"),
                &[("Authorization", &auth_hdr)],
                None,
            )
            .await;
        assert_eq!(st, 202);
        let cmd = find_command(id).await;
        assert_eq!(cmd.fingerprint, fp);
        assert!(matches!(cmd.kind, AlertCommandKind::Ack));
        assert_eq!(
            cmd.actor,
            Actor::User {
                person_id: "ala".into()
            }
        );
        s.cleanup().await;
    }

    #[tokio::test]
    async fn resolve_accepted() {
        let s = server().await;
        let id = s.insert_alert(&uniq("fp"), "open", 0).await;
        sqlx::query("INSERT INTO people (id, name, contacts, def) VALUES ('bob', 'Bob', '{}'::jsonb, '{}'::jsonb) ON CONFLICT (id) DO NOTHING")
            .execute(&s.st.db)
            .await
            .unwrap();
        let sess = s.st.auth.create_session("bob".into());
        let auth_hdr = format!("Bearer {sess}");
        let (st, _, _) = s
            .http_raw(
                "POST",
                &format!("/alerts/{id}/resolve"),
                &[("Authorization", &auth_hdr)],
                None,
            )
            .await;
        assert_eq!(st, 202);
        let (st, _) = s
            .http("POST", &format!("/alerts/{id}/resolve"), &[], None)
            .await;
        assert_eq!(st, 403);
        s.cleanup().await;
    }

    #[tokio::test]
    async fn ingest_errors_listed() {
        let s = server().await;
        let src = uniq("src");
        sqlx::query(
            "INSERT INTO ingest_errors (source, error, sample) VALUES ($1, 'zly json', '{')",
        )
        .bind(&src)
        .execute(&s.st.db)
        .await
        .unwrap();
        let (st, v) = s.http("GET", "/ingest/errors", &[], None).await;
        assert_eq!(st, 200);
        let e = v
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["source"] == src.as_str())
            .expect("brak błędu");
        assert_eq!(e["error"], "zly json");
        assert_eq!(e["sample"], "{");
        sqlx::query("DELETE FROM ingest_errors WHERE source = $1")
            .bind(&src)
            .execute(&s.st.db)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn auth_rules() {
        let s = server().await;
        let tok = s.st.config.api_token.clone();
        let (st, _, _) = s.http_raw("GET", "/health", &[], None).await;
        assert_eq!(st, 200);

        let (st, _, v) = s.http_raw("GET", "/alerts", &[], None).await;
        assert_eq!((st, &v), (401, &json!({"error": "unauthorized"})));
        let (st, _, _) = s
            .http_raw("GET", "/alerts", &[("Authorization", "Bearer zly")], None)
            .await;
        assert_eq!(st, 401);
        let (st, _, _) = s
            .http_raw(
                "GET",
                "/alerts?limit=1",
                &[("Authorization", &format!("Bearer {tok}"))],
                None,
            )
            .await;
        assert_eq!(st, 200);

        let (st, _, _) = s
            .http_raw("GET", &format!("/alerts?limit=1&token={tok}"), &[], None)
            .await;
        assert_eq!(st, 401);
        let (st, _, _) = s.http_raw("GET", "/alerts?token=zly", &[], None).await;
        assert_eq!(st, 401);

        let (st, _) = s.head_only("/stream").await;
        assert_eq!(st, 401);
        let (st, head) = s.head_only(&format!("/stream?token={tok}")).await;
        assert_eq!(st, 200);
        assert!(head.contains("content-type: text/event-stream"), "{head}");

        let (st, head, _) = s
            .http_raw(
                "OPTIONS",
                "/alerts",
                &[
                    ("Origin", "http://localhost:5173"),
                    ("Access-Control-Request-Method", "GET"),
                ],
                None,
            )
            .await;
        assert_ne!(st, 401);
        assert!(head.contains("access-control-allow-origin"), "{head}");
    }

    #[tokio::test]
    async fn list_alerts_team_filter_and_presentation() {
        let s = server().await;
        let rule_id = uniq("d10-rule");
        let team_id = uniq("d10-team");
        let fp_match = uniq("d10-fp-match");
        let fp_other = uniq("d10-fp-other");

        let mut alert_changed: contracts::AlertChanged = serde_json::from_str(include_str!(
            "../../../contracts/fixtures/topics/alerts.changed.json"
        ))
        .unwrap();
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

        let (st, list) = s
            .http("GET", &format!("/alerts?team={team_id}"), &[], None)
            .await;
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

        let (st, detail) = s
            .http("GET", &format!("/alerts/{id_match}"), &[], None)
            .await;
        assert_eq!(st, 200);
        assert_eq!(detail["body"], "Step validate-counterparty");
        assert_eq!(detail["fields"][0]["path"], "raw.step");
        assert_eq!(detail["fields"][0]["highlight"], "red");

        sqlx::query("DELETE FROM rules WHERE id = $1")
            .bind(&rule.id)
            .execute(&s.st.db)
            .await
            .unwrap();
        s.cleanup().await;
    }
}
