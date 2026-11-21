//! Runbooks, step commands, approvals, and audit log routes.
//!
//! Reads go straight to the runbook tables; step state mutations are published exclusively
//! as `RunbookCommand` messages on `runbooks.commands` (single writer = `runbooks` service).

use std::collections::HashSet;

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
use contracts::{topics, ActionDef, Actor, Condition, RunbookCommand, RunbookCommandKind, RunbookDef, RunbookKind, StepDef};
use runbooks::author::{AuthorError, AuthorStatus, DraftOutcome, DraftRequest};
use runbooks::registry::Registry;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{db_err, err, parse_uuid, ApiResult};
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/runbooks", get(list_runbooks).post(create_runbook))
        .route("/runbooks/author/status", get(author_status))
        .route("/runbooks/draft", post(draft_runbook))
        .route("/runbooks/errors", get(runbook_errors))
        .route("/alerts/{alert_id}/runbooks", get(alert_runbooks))
        .route("/alerts/{alert_id}/audit", get(alert_audit))
        .route("/runs/{run_id}/steps/{step_id}/{command}", post(step_command))
        .route("/approvals", get(approvals))
}

#[derive(Debug, Deserialize)]
struct DraftApiRequest {
    runbook_id: Option<String>,
    id: Option<String>,
    title: Option<String>,
    text: Option<String>,
    matches: Option<Condition>,
    #[serde(default)]
    rule_ids: Vec<String>,
}

async fn author_status(State(st): State<AppState>) -> Json<AuthorStatus> {
    Json(st.author.status())
}

fn map_author_error(e: AuthorError) -> (StatusCode, Json<Value>) {
    match e {
        AuthorError::Disabled => err(StatusCode::NOT_IMPLEMENTED, "llm author disabled", e),
        AuthorError::InvalidInput(msg) => err(StatusCode::BAD_REQUEST, "invalid draft request", msg),
        AuthorError::Timeout(_) => err(StatusCode::GATEWAY_TIMEOUT, "llm timeout", e),
        AuthorError::Upstream(_) | AuthorError::InvalidResponse(_) => err(StatusCode::BAD_GATEWAY, "llm error", e),
    }
}

async fn load_actions(st: &AppState) -> ApiResult<Vec<ActionDef>> {
    let rows: Vec<(Value, i64)> = sqlx::query_as("SELECT definition, version FROM actions ORDER BY name")
        .fetch_all(&st.db)
        .await
        .map_err(db_err)?;
    Ok(rows
        .into_iter()
        .filter_map(|(d, v)| serde_json::from_value::<ActionDef>(d).ok().map(|a| ActionDef { version: v, ..a }))
        .collect())
}

async fn draft_runbook(
    State(st): State<AppState>,
    body: Result<Json<DraftApiRequest>, JsonRejection>,
) -> ApiResult<Json<DraftOutcome>> {
    if !st.author.is_enabled() {
        return Err(map_author_error(AuthorError::Disabled));
    }
    let Json(req) = body.map_err(|e| err(StatusCode::BAD_REQUEST, "invalid draft request", e.body_text()))?;

    let mut id = req.id.filter(|s| !s.trim().is_empty());
    let mut title = req.title.filter(|s| !s.trim().is_empty());
    let mut text = req.text.filter(|s| !s.trim().is_empty());
    let mut matches = req.matches;
    let mut rule_ids = req.rule_ids;

    if let Some(rb_id) = req.runbook_id.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let row: Option<(Value,)> =
            sqlx::query_as("SELECT COALESCE(resolved, definition) FROM runbooks WHERE id = $1").bind(rb_id).fetch_optional(&st.db).await.map_err(db_err)?;
        let (raw,) = row.ok_or_else(|| err(StatusCode::NOT_FOUND, "runbook not found", rb_id))?;
        let rb: RunbookDef =
            serde_json::from_value(raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, "invalid stored runbook", e))?;
        if text.is_none() {
            text = rb.body.filter(|b| !b.trim().is_empty());
        }
        if id.is_none() {
            id = Some(format!("{}-auto", rb.id));
        }
        if title.is_none() {
            title = Some(rb.title);
        }
        if matches.is_none() {
            matches = rb.matches;
        }
        if rule_ids.is_empty() {
            rule_ids = rb.rule_ids;
        }
    }

    let text = text.ok_or_else(|| err(StatusCode::BAD_REQUEST, "invalid draft request", "podaj niepusty `text` lub `runbook_id` z treścią"))?;
    let actions = load_actions(&st).await?;
    let draft_req = DraftRequest { id, title, text, matches, rule_ids };
    let outcome = st.author.draft(&draft_req, &actions).await.map_err(map_author_error)?;
    Ok(Json(outcome))
}

fn validate_runbook_def(def: &RunbookDef) -> Result<(), String> {
    if def.id.is_empty() || !def.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(format!("niepoprawne id runbooka `{}` (dozwolone: litery, cyfry, _ i -)", def.id));
    }
    if def.title.trim().is_empty() {
        return Err("tytuł runbooka nie może być pusty".into());
    }
    if def.kind != RunbookKind::Automatic {
        return Err("przez API można zapisywać wyłącznie runbooki `kind: automatic`".into());
    }
    if def.steps.is_empty() {
        return Err("runbook musi zawierać co najmniej jeden krok".into());
    }
    let mut seen = HashSet::new();
    for (i, step) in def.steps.iter().enumerate() {
        if step.id.trim().is_empty() {
            return Err(format!("krok {}: brak id", i + 1));
        }
        if !seen.insert(step.id.as_str()) {
            return Err(format!("krok {}: powtórzone id `{}`", i + 1, step.id));
        }
        if step.text.trim().is_empty() {
            return Err(format!("krok {}: brak opisu `text`", step.id));
        }
    }
    Ok(())
}

async fn create_runbook(
    State(st): State<AppState>,
    body: Result<Json<RunbookDef>, JsonRejection>,
) -> ApiResult<(StatusCode, Json<RunbookDef>)> {
    let Json(mut def) = body.map_err(|e| err(StatusCode::BAD_REQUEST, "invalid runbook", e.body_text()))?;
    validate_runbook_def(&def).map_err(|e| err(StatusCode::BAD_REQUEST, "invalid runbook", e))?;
    def.source_file = None;
    def.body = None;
    def.version = 0;

    let existing: Option<Option<String>> =
        sqlx::query_scalar("SELECT source_file FROM runbooks WHERE id = $1").bind(&def.id).fetch_optional(&st.db).await.map_err(db_err)?;
    if let Some(source) = existing {
        let detail = match source {
            Some(file) => format!("runbook `{}` pochodzi z pliku {file}; wybierz inne id", def.id),
            None => format!("runbook `{}` już istnieje", def.id),
        };
        return Err(err(StatusCode::CONFLICT, "runbook already exists", detail));
    }

    let actions = load_actions(&st).await?;
    let custom_actions: Vec<ActionDef> = actions.into_iter().filter(|a| a.kind != contracts::ActionKind::Builtin).collect();
    let resolved = Registry::new(custom_actions).classify(&def);
    let def_val = serde_json::to_value(&def).map_err(|e| err(StatusCode::BAD_REQUEST, "invalid runbook", e))?;
    let res_val = serde_json::to_value(&resolved).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, "invalid runbook", e))?;

    let version: Option<i64> = sqlx::query_scalar(
        "INSERT INTO runbooks (id, definition, resolved, source_file) VALUES ($1, $2, $3, NULL)
         ON CONFLICT (id) DO NOTHING RETURNING version",
    )
    .bind(&def.id)
    .bind(&def_val)
    .bind(&res_val)
    .fetch_optional(&st.db)
    .await
    .map_err(db_err)?;
    let version = version.ok_or_else(|| err(StatusCode::CONFLICT, "runbook already exists", &def.id))?;
    Ok((StatusCode::CREATED, Json(RunbookDef { version, ..resolved })))
}

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

/// Extracts line number from serde_yaml errors: `"... at line 3 column 1 ..."` -> `Some(3)`.
fn error_line(error: &str) -> Option<i64> {
    let rest = &error[error.find("line ")? + 5..];
    rest.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
}

async fn runbook_errors(State(st): State<AppState>) -> ApiResult<Json<Value>> {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT source_file, error FROM runbook_load_errors ORDER BY source_file").fetch_all(&st.db).await.map_err(db_err)?;
    let out: Vec<Value> = rows.into_iter().map(|(file, error)| json!({ "file": file, "line": error_line(&error), "error": error })).collect();
    Ok(Json(json!(out)))
}

/// Runbooks for the alert's current episode, ordered by specificity, with step states.
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

/// Converts `StepDef` into OpenAPI `StepState` shape (`id` -> `step_id`).
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

/// Checks whether a command is valid for a step in the given status (mirrored in `runbooks`).
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
    axum::Extension(auth): axum::Extension<crate::auth::AuthenticatedOperator>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let run_id = parse_uuid(&run_id, "run_id")?;
    let person_id = crate::auth::resolve_verified_person_id(&st, &auth, &headers).await?;
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
    fn validates_runbook_def_for_create() {
        let mut rb = RunbookDef {
            id: "payments-auto".into(),
            title: "Payments auto".into(),
            kind: RunbookKind::Automatic,
            matches: None,
            rule_ids: vec![],
            body: None,
            steps: vec![StepDef {
                id: "s1".into(),
                step_type: contracts::StepType::Manual,
                text: "Check logs".into(),
                action: None,
                params: Default::default(),
                approval: contracts::Approval::None,
                condition: None,
                automation: contracts::Automation::Manual,
            }],
            source_file: None,
            version: 0,
        };
        assert!(validate_runbook_def(&rb).is_ok());

        rb.id = "bad id!".into();
        assert!(validate_runbook_def(&rb).is_err());

        rb.id = "payments-auto".into();
        rb.steps.clear();
        assert!(validate_runbook_def(&rb).is_err());
    }

    #[test]
    fn maps_author_errors_to_expected_http_statuses() {
        assert_eq!(map_author_error(AuthorError::Disabled).0, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(map_author_error(AuthorError::InvalidInput("x".into())).0, StatusCode::BAD_REQUEST);
        assert_eq!(map_author_error(AuthorError::Timeout(5000)).0, StatusCode::GATEWAY_TIMEOUT);
        assert_eq!(map_author_error(AuthorError::Upstream("x".into())).0, StatusCode::BAD_GATEWAY);
    }
}
