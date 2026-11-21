//! Actions CRUD and missing actions routes.

use axum::{
    extract::{rejection::JsonRejection, Path, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use common::db::sqlx;
use contracts::{ActionDef, ActionKind};
use serde_json::{json, Value};

use crate::error::{db_err, err, ApiResult};
use crate::AppState;

/// Built-in actions defined by the `runbooks` service; cannot be overwritten or deleted.
const BUILTIN_ACTIONS: [&str; 3] = ["http_get", "escalate", "notify"];
const METHODS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/actions", get(list_actions).post(create_action))
        .route("/actions/missing", get(missing_actions))
        .route("/actions/{name}", get(get_action).put(put_action).delete(delete_action))
}

/// Same validation as `runbooks::registry::validate_action` (templates are validated by `condition`).
fn validate_action(def: &ActionDef) -> Result<(), String> {
    if def.name.is_empty() || !def.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(format!("niepoprawna nazwa akcji `{}` (dozwolone: litery, cyfry, _ i -)", def.name));
    }
    if BUILTIN_ACTIONS.contains(&def.name.as_str()) {
        return Err(format!("`{}` to akcja wbudowana; wybierz inną nazwę", def.name));
    }
    if def.kind != ActionKind::Http {
        return Err("można dodawać tylko akcje `kind: http`".into());
    }
    let http = def.http.as_ref().ok_or("brak sekcji `http`")?;
    if !METHODS.contains(&http.method.as_str()) {
        return Err(format!("metoda `{}` niedozwolona (dozwolone: {})", http.method, METHODS.join(", ")));
    }
    if !(http.url.starts_with("http://") || http.url.starts_with("https://")) {
        return Err("url musi zaczynać się od http:// albo https://".into());
    }
    if !(1..=30_000).contains(&http.timeout_ms) {
        return Err("timeout_ms musi być w zakresie 1..=30000".into());
    }
    Ok(())
}

fn action_from_row(definition: Value, version: i64) -> ApiResult<ActionDef> {
    let def: ActionDef = serde_json::from_value(definition).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, "invalid stored action", e))?;
    Ok(ActionDef { version, ..def })
}

async fn list_actions(State(st): State<AppState>) -> ApiResult<Json<Vec<ActionDef>>> {
    let rows: Vec<(Value, i64)> = sqlx::query_as("SELECT definition, version FROM actions ORDER BY name").fetch_all(&st.db).await.map_err(db_err)?;
    Ok(Json(rows.into_iter().filter_map(|(d, v)| action_from_row(d, v).ok()).collect()))
}

async fn get_action(State(st): State<AppState>, Path(name): Path<String>) -> ApiResult<Json<ActionDef>> {
    let (definition, version): (Value, i64) = sqlx::query_as("SELECT definition, version FROM actions WHERE name = $1")
        .bind(&name)
        .fetch_optional(&st.db)
        .await
        .map_err(db_err)?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "action not found", &name))?;
    Ok(Json(action_from_row(definition, version)?))
}

fn parse_action_body(body: Result<Json<ActionDef>, JsonRejection>) -> ApiResult<ActionDef> {
    let Json(mut def) = body.map_err(|e| err(StatusCode::BAD_REQUEST, "invalid action", e.body_text()))?;
    validate_action(&def).map_err(|e| err(StatusCode::BAD_REQUEST, "invalid action", e))?;
    def.version = 0;
    Ok(def)
}

async fn create_action(State(st): State<AppState>, body: Result<Json<ActionDef>, JsonRejection>) -> ApiResult<(StatusCode, Json<ActionDef>)> {
    let def = parse_action_body(body)?;
    let stored = serde_json::to_value(&def).map_err(|e| err(StatusCode::BAD_REQUEST, "invalid action", e))?;
    let version: Option<i64> =
        sqlx::query_scalar("INSERT INTO actions (name, definition) VALUES ($1, $2) ON CONFLICT (name) DO NOTHING RETURNING version")
            .bind(&def.name)
            .bind(&stored)
            .fetch_optional(&st.db)
            .await
            .map_err(db_err)?;
    let version = version.ok_or_else(|| err(StatusCode::CONFLICT, "action already exists", &def.name))?;
    Ok((StatusCode::CREATED, Json(ActionDef { version, ..def })))
}

async fn put_action(
    State(st): State<AppState>,
    Path(name): Path<String>,
    body: Result<Json<ActionDef>, JsonRejection>,
) -> ApiResult<Json<ActionDef>> {
    let def = parse_action_body(body)?;
    if def.name != name {
        return Err(err(StatusCode::BAD_REQUEST, "name mismatch", "name w body różni się od ścieżki"));
    }
    let stored = serde_json::to_value(&def).map_err(|e| err(StatusCode::BAD_REQUEST, "invalid action", e))?;
    let version: i64 = sqlx::query_scalar(
        "INSERT INTO actions (name, definition) VALUES ($1, $2)
         ON CONFLICT (name) DO UPDATE SET definition = EXCLUDED.definition, version = actions.version + 1, updated_at = now()
         RETURNING version",
    )
    .bind(&name)
    .bind(&stored)
    .fetch_one(&st.db)
    .await
    .map_err(db_err)?;
    Ok(Json(ActionDef { version, ..def }))
}

async fn delete_action(State(st): State<AppState>, Path(name): Path<String>) -> ApiResult<StatusCode> {
    let source: Option<Option<String>> =
        sqlx::query_scalar("SELECT source_file FROM actions WHERE name = $1").bind(&name).fetch_optional(&st.db).await.map_err(db_err)?;
    if let Some(Some(file)) = source {
        // File-defined actions would be recreated on the next reload.
        return Err(err(StatusCode::CONFLICT, "action defined in a file", format!("akcja pochodzi z {file}; usuń plik")));
    }
    sqlx::query("DELETE FROM actions WHERE name = $1").bind(&name).execute(&st.db).await.map_err(db_err)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn missing_actions(State(st): State<AppState>) -> ApiResult<Json<Value>> {
    let rows: Vec<(String, Vec<String>, i64)> = sqlx::query_as(
        "SELECT u.action, u.runbook_ids,
                (SELECT count(*) FROM step_runs s JOIN runbook_runs r USING (run_id)
                 CROSS JOIN LATERAL jsonb_array_elements(r.steps) st
                 WHERE st->>'id' = s.step_id AND st->>'action' = u.action
                   AND s.status = 'succeeded' AND s.actor->>'type' = 'user')
         FROM unhandled_actions u ORDER BY u.occurrences DESC, u.action",
    )
    .fetch_all(&st.db)
    .await
    .map_err(db_err)?;
    let out: Vec<Value> =
        rows.into_iter().map(|(name, runbook_ids, n)| json!({ "name": name, "runbook_ids": runbook_ids, "manual_executions": n })).collect();
    Ok(Json(json!(out)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_names_and_non_http_rejected() {
        let mut def: ActionDef = serde_json::from_value(json!({
            "name": "escalate", "kind": "http", "risk": "low",
            "http": { "method": "POST", "url": "http://mock-targets:8090/x" }
        }))
        .unwrap();
        assert!(validate_action(&def).unwrap_err().contains("wbudowana"));
        def.name = "flush_cdn".into();
        assert!(validate_action(&def).is_ok());
        def.kind = ActionKind::Builtin;
        assert!(validate_action(&def).is_err());
    }
}
