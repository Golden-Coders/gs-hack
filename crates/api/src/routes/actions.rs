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

const FORBIDDEN_ENV_EXACT: &[&str] = &[
    "DATABASE_URL",
    "API_TOKEN",
    "LLM_API_KEY",
    "LLM_PROVIDER",
    "LLM_MODEL",
    "LLM_BASE_URL",
    "POSTGRES_PASSWORD",
    "POSTGRES_USER",
    "POSTGRES_DB",
    "KAFKA_BROKERS",
    "SMTP_HOST",
    "SMTP_PORT",
    "ACTION_HOST_ALLOWLIST",
    "PUBLIC_UI_URL",
    "CONFIG_DIR",
    "OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
    "GEMINI_API_KEY",
    "GITHUB_TOKEN",
    "PATH",
    "HOME",
    "USER",
    "PWD",
    "SHELL",
    "RUST_LOG",
];

const FORBIDDEN_ENV_PREFIXES: &[&str] = &[
    "DATABASE_",
    "POSTGRES_",
    "KAFKA_",
    "SMTP_",
    "LLM_",
    "OPENAI_",
    "ANTHROPIC_",
    "GEMINI_",
    "AWS_",
    "GOOGLE_",
    "AZURE_",
    "GCP_",
    "API_",
];

fn validate_header_env_refs(header_name: &str, value: &str) -> Result<(), String> {
    let mut rest = value;
    while let Some(start) = rest.find("${") {
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            return Err(format!("nagłówek `{header_name}` zawiera niedomkniętą referencję `${{...`"));
        };
        let var = &after[..end];
        if var.is_empty()
            || !var.as_bytes()[0].is_ascii_uppercase()
            || !var.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        {
            return Err(format!("nagłówek `{header_name}` zawiera niepoprawną nazwę zmiennej `{var}`"));
        }
        let is_forbidden = FORBIDDEN_ENV_EXACT.contains(&var)
            || FORBIDDEN_ENV_PREFIXES.iter().any(|p| var.starts_with(p));
        let is_allowed_naming = var.starts_with("ACTION_")
            || var.ends_with("_TOKEN")
            || var.ends_with("_SECRET")
            || var.ends_with("_KEY");
        if is_forbidden || !is_allowed_naming {
            return Err(format!("niedozwolona referencja sekretu `${{{var}}}` w nagłówku `{header_name}`"));
        }
        rest = &after[end + 1..];
    }
    Ok(())
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
    condition::validate_template(&http.url).map_err(|e| format!("url: {e}"))?;
    if let Some(body) = &http.body {
        condition::validate_template(body).map_err(|e| format!("body: {e}"))?;
    }
    for (k, v) in &http.headers {
        validate_header_env_refs(k, v)?;
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
    if BUILTIN_ACTIONS.contains(&name.as_str()) {
        return Err(err(StatusCode::CONFLICT, "builtin action", "nie można modyfikować akcji wbudowanej"));
    }
    let stored = serde_json::to_value(&def).map_err(|e| err(StatusCode::BAD_REQUEST, "invalid action", e))?;
    let mut tx = st.db.begin().await.map_err(db_err)?;

    if def.name != name {
        let conflict: Option<i32> = sqlx::query_scalar("SELECT 1 FROM actions WHERE name = $1")
            .bind(&def.name)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db_err)?;
        if conflict.is_some() {
            return Err(err(StatusCode::CONFLICT, "action already exists", &def.name));
        }
        sqlx::query("DELETE FROM actions WHERE name = $1")
            .bind(&name)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }

    let version: i64 = sqlx::query_scalar(
        "INSERT INTO actions (name, definition, source_file) VALUES ($1, $2, NULL)
         ON CONFLICT (name) DO UPDATE SET definition = EXCLUDED.definition, source_file = NULL, version = actions.version + 1, updated_at = now()
         RETURNING version",
    )
    .bind(&def.name)
    .bind(&stored)
    .fetch_one(&mut *tx)
    .await
    .map_err(db_err)?
    ;
    tx.commit().await.map_err(db_err)?;
    Ok(Json(ActionDef { version, ..def }))
}

async fn delete_action(State(st): State<AppState>, Path(name): Path<String>) -> ApiResult<StatusCode> {
    if BUILTIN_ACTIONS.contains(&name.as_str()) {
        return Err(err(StatusCode::CONFLICT, "builtin action", "nie można usunąć akcji wbudowanej"));
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

    #[test]
    fn templates_and_header_secrets_validated() {
        let valid: ActionDef = serde_json::from_value(json!({
            "name": "flush_cache", "kind": "http", "risk": "high",
            "http": {
                "method": "POST",
                "url": "http://mock-targets:8090/cache/{{service}}/flush",
                "headers": { "Authorization": "Bearer ${CACHE_ADMIN_TOKEN}" },
                "body": "{\"summary\": \"{{summary}}\"}"
            }
        }))
        .unwrap();
        assert!(validate_action(&valid).is_ok());

        let bad_url: ActionDef = serde_json::from_value(json!({
            "name": "bad_url", "kind": "http", "risk": "low",
            "http": { "method": "POST", "url": "http://mock-targets:8090/{{unclosed" }
        }))
        .unwrap();
        assert!(validate_action(&bad_url).is_err());

        let bad_body: ActionDef = serde_json::from_value(json!({
            "name": "bad_body", "kind": "http", "risk": "low",
            "http": { "method": "POST", "url": "http://mock-targets:8090/x", "body": "{{unclosed" }
        }))
        .unwrap();
        assert!(validate_action(&bad_body).is_err());

        for forbidden in ["DATABASE_URL", "API_TOKEN", "LLM_API_KEY", "POSTGRES_PASSWORD", "PATH", "AWS_SECRET_ACCESS_KEY"] {
            let def: ActionDef = serde_json::from_value(json!({
                "name": "leak_secret", "kind": "http", "risk": "low",
                "http": {
                    "method": "POST",
                    "url": "http://mock-targets:8090/x",
                    "headers": { "X-Leak": format!("${{{forbidden}}}") }
                }
            }))
            .unwrap();
            assert!(validate_action(&def).is_err(), "expected {forbidden} to be rejected");
        }
    }
}
