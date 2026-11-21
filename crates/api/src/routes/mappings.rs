//! Mappings CRUD, preview, and unmapped events routes.

use axum::{
    extract::{rejection::JsonRejection, Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use common::db::sqlx::{
    self,
    types::{chrono::DateTime, chrono::Utc, Json as SqlJson, Uuid},
    Row,
};
use contracts::{formats, CanonicalEvent, Envelope, MappingDef};
use lifecycle::mapping::{parse_payload, CompiledMapping};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{db_err, err, opt_json, ApiResult};
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/mappings", get(list_mappings))
        .route("/mappings/preview", post(preview))
        .route(
            "/mappings/{schema}",
            get(get_mapping).put(put_mapping).delete(delete_mapping),
        )
        .route("/unmapped", get(unmapped))
}

fn mapping_from_row(r: &sqlx::postgres::PgRow) -> ApiResult<MappingDef> {
    let mut def: MappingDef = serde_json::from_value(opt_json(r, "def")).map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid stored mapping",
            e,
        )
    })?;
    def.version = r.get::<i64, _>("version");
    Ok(def)
}

async fn list_mappings(State(st): State<AppState>) -> ApiResult<Json<Vec<MappingDef>>> {
    let rows = sqlx::query("SELECT def, version::bigint AS version FROM mappings ORDER BY schema")
        .fetch_all(&st.db)
        .await
        .map_err(db_err)?;
    rows.iter()
        .map(mapping_from_row)
        .collect::<ApiResult<Vec<_>>>()
        .map(Json)
}

async fn get_mapping(
    State(st): State<AppState>,
    Path(schema): Path<String>,
) -> ApiResult<Json<MappingDef>> {
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
    let Json(mut def) =
        body.map_err(|e| err(StatusCode::BAD_REQUEST, "invalid mapping", e.body_text()))?;
    if def.schema != schema {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "schema mismatch",
            "schema w body różni się od ścieżki",
        ));
    }
    CompiledMapping::compile(def.clone(), 0)
        .map_err(|e| err(StatusCode::BAD_REQUEST, "invalid mapping", format!("{e:#}")))?;
    let stored = serde_json::to_value(&def)
        .map_err(|e| err(StatusCode::BAD_REQUEST, "invalid mapping", e))?;
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
    sqlx::query("SELECT pg_notify('config_changed', 'mappings')")
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    def.version = version;
    Ok(Json(def))
}

async fn delete_mapping(
    State(st): State<AppState>,
    Path(schema): Path<String>,
) -> ApiResult<StatusCode> {
    let mut tx = st.db.begin().await.map_err(db_err)?;
    sqlx::query("DELETE FROM mappings WHERE schema = $1")
        .bind(&schema)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    sqlx::query("SELECT pg_notify('config_changed', 'mappings')")
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct PreviewReq {
    mapping: MappingDef,
    payload: Value,
}

/// Preview canonical event normalization without persisting anything.
/// String payloads are parsed as YAML (pasted in UI), everything else as JSON.
async fn preview(body: Result<Json<PreviewReq>, JsonRejection>) -> ApiResult<Json<CanonicalEvent>> {
    let bad = |e: String| err(StatusCode::BAD_REQUEST, "invalid preview", e);
    let Json(req) = body.map_err(|e| bad(e.body_text()))?;
    let format = if req.payload.is_string() {
        formats::YAML
    } else {
        formats::JSON
    };
    let raw = parse_payload(format, &req.payload).map_err(|e| bad(format!("{e:#}")))?;
    let version = req.mapping.version;
    let mapping =
        CompiledMapping::compile(req.mapping, version).map_err(|e| bad(format!("{e:#}")))?;
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
    use std::time::Duration;

    use common::db::sqlx::{
        self,
        types::{Json as SqlJson, Uuid},
    };
    use serde_json::{json, Value};

    use crate::routes::test_util::{server, uniq, Server};

    const WORKFLOW_MAPPING: &str =
        include_str!("../../../contracts/fixtures/config/mapping.workflow.json");
    const WORKFLOW_EVENT: &str =
        include_str!("../../../contracts/fixtures/events/workflow_failed.json");

    async fn mapping_rows(s: &Server, schema: &str) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM mappings WHERE schema = $1")
            .bind(schema)
            .fetch_one(&s.st.db)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn mappings_crud_and_notify() {
        let s = server().await;
        let schema = uniq("t-schema");
        let body = json!({"schema": schema, "severity": {"type": "path", "path": "$.s", "map": {}, "default": "warning"}}).to_string();
        let path = format!("/mappings/{schema}");

        let mut listener = sqlx::postgres::PgListener::connect_with(&s.st.db)
            .await
            .unwrap();
        listener.listen("config_changed").await.unwrap();

        let (st, v1) = s.http("PUT", &path, &[], Some(&body)).await;
        assert_eq!(st, 200);
        let (st, v2) = s.http("PUT", &path, &[], Some(&body)).await;
        assert_eq!(st, 200);
        assert_eq!(
            v2["version"].as_i64().unwrap(),
            v1["version"].as_i64().unwrap() + 1
        );

        let n = tokio::time::timeout(Duration::from_secs(5), listener.recv())
            .await
            .expect("brak NOTIFY")
            .unwrap();
        assert_eq!(n.channel(), "config_changed");
        assert_eq!(n.payload(), "mappings");

        let (st, v) = s.http("GET", &path, &[], None).await;
        assert_eq!(st, 200);
        assert_eq!(v["schema"], schema);
        let (_, list) = s.http("GET", "/mappings", &[], None).await;
        assert!(list
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["schema"] == schema.as_str()));

        let (st, _) = s
            .http("PUT", &path, &[], Some(&body.replace(&schema, "inny")))
            .await;
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
        let row = v
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["schema"] == schema.as_str())
            .expect("brak schematu");
        assert_eq!(row["count"], 3);
        assert_eq!(row["sample"], json!({"n": 2}));
        sqlx::query("DELETE FROM unmapped_events WHERE schema = $1")
            .bind(&schema)
            .execute(&s.st.db)
            .await
            .unwrap();
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
        assert_eq!(
            v["title"],
            "Counterparty service did not respond in 5000 ms"
        );
        assert_eq!(
            v["fingerprint"],
            "workflow.v1|order-settlement|validate-counterparty|TIMEOUT"
        );
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
        let (st, _) = s
            .http(
                "PUT",
                &format!("/mappings/{schema}"),
                &[],
                Some(&mapping.to_string()),
            )
            .await;
        assert_eq!(st, 400);
        assert_eq!(mapping_rows(&s, &schema).await, 0);
    }
}
