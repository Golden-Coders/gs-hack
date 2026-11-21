//! Routy strumienia P3: runbooki, akcje (CRUD, brakujące), wykonania kroków,
//! kolejka zatwierdzeń, dziennik. Właściciel: P3. Zob. `contracts/openapi.yaml` (tag `p3`).
//!
//! Odczyty idą prosto z tabel P3. Zmiany stanu kroków idą WYŁĄCZNIE jako `RunbookCommand`
//! na `runbooks.commands` (single writer = binarka `runbooks`). Akcje zapisujemy do tabeli
//! `actions`; `runbooks` przeładowuje rejestr w ciągu ~2 s.

use axum::{
    body::Bytes,
    extract::{rejection::JsonRejection, Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use common::db::sqlx::{
    self,
    types::{chrono::DateTime, chrono::Utc, Uuid},
    Row,
};
use contracts::{topics, ActionDef, ActionKind, Actor, RunbookCommand, RunbookCommandKind, RunbookDef, StepDef};
use serde_json::{json, Value};

use crate::AppState;

type ApiResult<T> = Result<T, (StatusCode, Json<Value>)>;

/// Akcje wbudowane (definiuje je binarka `runbooks`); nie da się ich nadpisać ani usunąć.
const BUILTIN_ACTIONS: [&str; 3] = ["http_get", "escalate", "notify"];
const METHODS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/runbooks", get(list_runbooks))
        .route("/runbooks/errors", get(runbook_errors))
        .route("/alerts/{alert_id}/runbooks", get(alert_runbooks))
        .route("/alerts/{alert_id}/audit", get(alert_audit))
        .route("/runs/{run_id}/steps/{step_id}/{command}", post(step_command))
        .route("/approvals", get(approvals))
        .route("/actions", get(list_actions).post(create_action))
        .route("/actions/missing", get(missing_actions))
        .route("/actions/{name}", get(get_action).put(put_action).delete(delete_action))
}

fn err(status: StatusCode, error: &str, detail: impl ToString) -> (StatusCode, Json<Value>) {
    (status, Json(json!({ "error": error, "detail": detail.to_string() })))
}

fn db_err(e: sqlx::Error) -> (StatusCode, Json<Value>) {
    tracing::error!(error = %e, "db error");
    err(StatusCode::INTERNAL_SERVER_ERROR, "database error", e)
}

fn parse_uuid(s: &str, what: &str) -> ApiResult<Uuid> {
    Uuid::parse_str(s).map_err(|e| err(StatusCode::BAD_REQUEST, &format!("invalid {what}"), e))
}

// ------------------------------------------------------------------ runbooki

async fn list_runbooks(State(st): State<AppState>) -> ApiResult<Json<Vec<RunbookDef>>> {
    let rows: Vec<(Value, i64)> = sqlx::query_as("SELECT COALESCE(resolved, definition), version FROM runbooks ORDER BY id")
        .fetch_all(&st.db)
        .await
        .map_err(db_err)?;
    let runbooks = rows
        .into_iter()
        .filter_map(|(def, version)| serde_json::from_value::<RunbookDef>(def).ok().map(|d| RunbookDef { version, ..d }))
        .collect();
    Ok(Json(runbooks))
}

/// „… at line 3 column 1 …” → 3 (błędy serde_yaml).
fn error_line(error: &str) -> Option<i64> {
    let rest = &error[error.find("line ")? + 5..];
    rest.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
}

async fn runbook_errors(State(st): State<AppState>) -> ApiResult<Json<Value>> {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT source_file, error FROM p3_load_errors ORDER BY source_file").fetch_all(&st.db).await.map_err(db_err)?;
    let out: Vec<Value> = rows.into_iter().map(|(file, error)| json!({ "file": file, "line": error_line(&error), "error": error })).collect();
    Ok(Json(json!(out)))
}

/// Runbooki bieżącego epizodu alertu, od najbardziej szczegółowego, ze stanem kroków.
async fn alert_runbooks(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let alert_id = parse_uuid(&id, "alert_id")?;
    let episode: Option<i32> = sqlx::query_scalar(
        "SELECT COALESCE((SELECT episode::int FROM alerts WHERE id = $1), (SELECT max(episode) FROM runbook_runs WHERE alert_id = $1))",
    )
    .bind(alert_id)
    .fetch_one(&st.db)
    .await
    .map_err(db_err)?;
    let Some(episode) = episode else { return Ok(Json(json!([]))) };

    let runs = sqlx::query(
        "SELECT run_id, runbook_id, title, kind, body, steps, created_at FROM runbook_runs
         WHERE alert_id = $1 AND episode = $2 ORDER BY specificity DESC, runbook_id",
    )
    .bind(alert_id)
    .bind(episode)
    .fetch_all(&st.db)
    .await
    .map_err(db_err)?;
    let run_ids: Vec<Uuid> = runs.iter().map(|r| r.get("run_id")).collect();
    let states = sqlx::query(
        "SELECT DISTINCT ON (run_id, step_id) run_id, step_id, status, attempt, result, note, updated_at
         FROM step_runs WHERE run_id = ANY($1) ORDER BY run_id, step_id, attempt DESC",
    )
    .bind(&run_ids)
    .fetch_all(&st.db)
    .await
    .map_err(db_err)?;

    let out: Vec<Value> = runs
        .iter()
        .map(|run| {
            let run_id: Uuid = run.get("run_id");
            let created_at: DateTime<Utc> = run.get("created_at");
            let steps: Vec<StepDef> = serde_json::from_value(run.get("steps")).unwrap_or_default();
            let steps: Vec<Value> = steps
                .iter()
                .map(|step| {
                    let state = states.iter().find(|s| s.get::<Uuid, _>("run_id") == run_id && s.get::<String, _>("step_id") == step.id);
                    let mut v = step_json(step);
                    v["status"] = json!(state.map_or("pending".to_string(), |s| s.get("status")));
                    v["attempt"] = json!(state.map_or(0, |s| s.get::<i32, _>("attempt")));
                    v["result"] = json!(state.and_then(|s| s.get::<Option<String>, _>("result")));
                    v["note"] = json!(state.and_then(|s| s.get::<Option<String>, _>("note")));
                    v["updated_at"] = json!(state.map_or(created_at, |s| s.get("updated_at")));
                    v
                })
                .collect();
            json!({
                "runbook_id": run.get::<String, _>("runbook_id"),
                "title": run.get::<String, _>("title"),
                "kind": run.get::<String, _>("kind"),
                "body": run.get::<Option<String>, _>("body"),
                "run_id": run_id,
                "episode": episode,
                "steps": steps,
            })
        })
        .collect();
    Ok(Json(json!(out)))
}

/// `StepDef` w kształcie `StepState` z OpenAPI (`id` → `step_id`).
fn step_json(step: &StepDef) -> Value {
    let mut v = serde_json::to_value(step).unwrap_or_default();
    if let Value::Object(map) = &mut v {
        if let Some(id) = map.remove("id") {
            map.insert("step_id".into(), id);
        }
    }
    v
}

async fn alert_audit(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let alert_id = parse_uuid(&id, "alert_id")?;
    let rows = sqlx::query("SELECT id, alert_id, episode, run_id, step_id, actor, action, detail, at FROM audit_log WHERE alert_id = $1 ORDER BY id")
        .bind(alert_id)
        .fetch_all(&st.db)
        .await
        .map_err(db_err)?;
    let out: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<i64, _>("id"),
                "alert_id": r.get::<Uuid, _>("alert_id"),
                "episode": r.get::<i32, _>("episode"),
                "run_id": r.get::<Option<Uuid>, _>("run_id"),
                "step_id": r.get::<Option<String>, _>("step_id"),
                "actor": r.get::<Value, _>("actor"),
                "action": r.get::<String, _>("action"),
                "detail": r.get::<Option<String>, _>("detail"),
                "at": r.get::<DateTime<Utc>, _>("at"),
            })
        })
        .collect();
    Ok(Json(json!(out)))
}

// ------------------------------------------------------------------ komendy i zatwierdzenia

/// Czy komenda ma sens dla kroku w tym stanie (ta sama reguła egzekwowana w `runbooks`).
fn command_allowed(kind: &RunbookCommandKind, status: &str) -> bool {
    match kind {
        RunbookCommandKind::Approve | RunbookCommandKind::Reject => status == "waiting_approval",
        RunbookCommandKind::Done { .. } => matches!(status, "waiting_manual" | "failed"),
        RunbookCommandKind::Retry => status == "failed",
        RunbookCommandKind::Skip => matches!(status, "failed" | "waiting_manual" | "waiting_approval"),
        RunbookCommandKind::ManualMode => matches!(status, "pending" | "failed" | "waiting_manual" | "waiting_approval"),
    }
}

async fn step_command(
    State(st): State<AppState>,
    Path((run_id, step_id, command)): Path<(String, String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let run_id = parse_uuid(&run_id, "run_id")?;
    let person_id = headers
        .get("x-person-id")
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "missing X-Person-Id header", "wymagany nagłówek X-Person-Id"))?
        .to_string();
    let note = if body.is_empty() {
        None
    } else {
        let v: Value = serde_json::from_slice(&body).map_err(|e| err(StatusCode::BAD_REQUEST, "invalid body", e))?;
        v.get("note").and_then(Value::as_str).map(str::to_string)
    };
    let kind = match command.as_str() {
        "approve" => RunbookCommandKind::Approve,
        "reject" => RunbookCommandKind::Reject,
        "done" => RunbookCommandKind::Done { note },
        "retry" => RunbookCommandKind::Retry,
        "skip" => RunbookCommandKind::Skip,
        "manual_mode" => RunbookCommandKind::ManualMode,
        other => return Err(err(StatusCode::BAD_REQUEST, "unknown command", other)),
    };

    let row: Option<(Uuid, Option<String>)> = sqlx::query_as(
        "SELECT r.alert_id, (SELECT status FROM step_runs WHERE run_id = r.run_id AND step_id = $2 ORDER BY attempt DESC LIMIT 1)
         FROM runbook_runs r WHERE r.run_id = $1",
    )
    .bind(run_id)
    .bind(&step_id)
    .fetch_optional(&st.db)
    .await
    .map_err(db_err)?;
    let (alert_id, status) = row.ok_or_else(|| err(StatusCode::NOT_FOUND, "run not found", run_id))?;
    let status = status.ok_or_else(|| err(StatusCode::NOT_FOUND, "step not found", &step_id))?;
    if !command_allowed(&kind, &status) {
        return Err(err(StatusCode::CONFLICT, "step state does not allow this command", format!("krok {step_id} ma status {status}")));
    }

    let cmd = RunbookCommand { alert_id, run_id, step_id, kind, actor: Actor::User { person_id }, at: Utc::now() };
    common::kafka::publish(&st.kafka, topics::RUNBOOKS_COMMANDS, &alert_id.to_string(), &cmd)
        .await
        .map_err(|e| err(StatusCode::SERVICE_UNAVAILABLE, "kafka unavailable", e))?;
    Ok(StatusCode::ACCEPTED)
}

async fn approvals(State(st): State<AppState>) -> ApiResult<Json<Value>> {
    let rows = sqlx::query(
        "SELECT r.alert_id, r.alert_title, r.run_id, r.title AS runbook_title, r.steps, s.step_id, s.risk, s.updated_at
         FROM step_runs s JOIN runbook_runs r USING (run_id)
         WHERE s.status = 'waiting_approval'
           AND s.attempt = (SELECT max(attempt) FROM step_runs x WHERE x.run_id = s.run_id AND x.step_id = s.step_id)
         ORDER BY s.updated_at",
    )
    .fetch_all(&st.db)
    .await
    .map_err(db_err)?;
    let out: Vec<Value> = rows
        .iter()
        .map(|r| {
            let step_id: String = r.get("step_id");
            let steps: Vec<StepDef> = serde_json::from_value(r.get("steps")).unwrap_or_default();
            let step = steps.iter().find(|s| s.id == step_id);
            json!({
                "alert_id": r.get::<Uuid, _>("alert_id"),
                "alert_title": r.get::<String, _>("alert_title"),
                "run_id": r.get::<Uuid, _>("run_id"),
                "runbook_title": r.get::<String, _>("runbook_title"),
                "step_id": step_id,
                "text": step.map_or(String::new(), |s| s.text.clone()),
                "action": step.and_then(|s| s.action.clone()),
                "params": step.map_or(json!({}), |s| json!(s.params)),
                "risk": r.get::<Option<String>, _>("risk").unwrap_or_else(|| "low".into()),
                "waiting_since": r.get::<DateTime<Utc>, _>("updated_at"),
            })
        })
        .collect();
    Ok(Json(json!(out)))
}

// ------------------------------------------------------------------ akcje

/// Ta sama walidacja co `runbooks::registry::validate_action` (bez szablonów – to robi `condition`).
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
        // Plik i tak wróciłby przy kolejnym przeładowaniu.
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
    fn yaml_error_line_extracted() {
        assert_eq!(error_line("x.yaml: did not find expected ',' at line 3 column 1, while parsing"), Some(3));
        assert_eq!(error_line("x.yaml: brak listy `steps`"), None);
    }

    #[test]
    fn commands_only_from_matching_states() {
        assert!(command_allowed(&RunbookCommandKind::Approve, "waiting_approval"));
        assert!(!command_allowed(&RunbookCommandKind::Approve, "succeeded"));
        assert!(command_allowed(&RunbookCommandKind::Done { note: None }, "waiting_manual"));
        assert!(!command_allowed(&RunbookCommandKind::Retry, "waiting_manual"));
    }

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
