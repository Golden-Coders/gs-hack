//! Rules CRUD, rule test sandbox, and field suggestion routes.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::{
    body::Bytes,
    extract::{rejection::JsonRejection, Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use common::db::sqlx::{
    self,
    types::{chrono::DateTime, chrono::Utc, Json as SqlJson, Uuid},
    PgPool, Row,
};
use contracts::{CanonicalEvent, MappingDef, Rendered, RuleDef, Severity};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::error::{db_err, err, err_with_line, opt_json, ApiResult};
use crate::routes::resolve_config_path;
use crate::AppState;

static RULES_SEEDED: AtomicBool = AtomicBool::new(false);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/rules", get(list_rules).post(create_rule))
        .route("/rules/draft", post(draft_rule))
        .route("/rules/test", post(test_rule))
        .route("/rules/{rule_id}", get(get_rule).put(put_rule).delete(delete_rule))
        .route("/fields", get(get_fields))
}

async fn ensure_rules_seeded(db: &PgPool, config_dir: &str) {
    if RULES_SEEDED.swap(true, Ordering::SeqCst) {
        return;
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rules")
        .fetch_one(db)
        .await
        .unwrap_or(1);
    if count > 0 {
        return;
    }
    let Some(rules_dir) = resolve_config_path(config_dir, "rules") else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(&rules_dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|x| x.path()))
        .filter(|p| {
            p.extension()
                .and_then(|s| s.to_str())
                .is_some_and(|ext| ext == "yaml" || ext == "yml")
        })
        .collect();
    paths.sort();

    for path in paths {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(mut rule) = serde_yaml::from_str::<RuleDef>(&text) else {
            continue;
        };
        if validate_rule(&rule).is_err() {
            continue;
        }
        if rule.version <= 0 {
            rule.version = 1;
        }
        let Ok(def_val) = serde_json::to_value(&rule) else {
            continue;
        };
        let _ = sqlx::query(
            "INSERT INTO rules (id, name, team, def, enabled, version, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, now()) ON CONFLICT (id) DO NOTHING",
        )
        .bind(&rule.id)
        .bind(&rule.name)
        .bind(&rule.team)
        .bind(SqlJson(def_val))
        .bind(rule.enabled)
        .bind(rule.version)
        .execute(db)
        .await;
    }
}

/// Detects unquoted YAML anchor (`&name`) or alias (`*name`) syntax to mitigate alias bombs.
fn contains_yaml_anchor_or_alias(text: &str) -> Option<usize> {
    for (idx, line) in text.lines().enumerate() {
        let mut in_single = false;
        let mut in_double = false;
        let mut escaped = false;
        let mut prev: Option<char> = None;
        let mut chars = line.chars().peekable();
        while let Some(ch) = chars.next() {
            if in_double {
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    in_double = false;
                }
                prev = Some(ch);
                continue;
            }
            if in_single {
                if ch == '\'' {
                    if chars.peek() == Some(&'\'') {
                        chars.next();
                    } else {
                        in_single = false;
                    }
                }
                prev = Some(ch);
                continue;
            }
            if ch == '#' && prev.is_none_or(char::is_whitespace) {
                break;
            }
            if ch == '"' {
                in_double = true;
            } else if ch == '\'' {
                in_single = true;
            } else if matches!(ch, '&' | '*')
                && prev.is_none_or(|p| p.is_whitespace() || matches!(p, ':' | '-' | '[' | '{' | ','))
                && chars
                    .peek()
                    .is_some_and(|n| !n.is_whitespace() && !matches!(n, ',' | ']' | '}' | ':'))
            {
                return Some(idx + 1);
            }
            prev = Some(ch);
        }
    }
    None
}

/// Parses a rule request body (JSON or YAML) and includes the line number on syntax errors.
pub fn parse_rule_payload(bytes: &[u8]) -> ApiResult<RuleDef> {
    let text = std::str::from_utf8(bytes)
        .map_err(|e| err_with_line(StatusCode::BAD_REQUEST, "invalid rule", e, Some(1)))?;
    if text.trim().is_empty() {
        return Err(err_with_line(
            StatusCode::BAD_REQUEST,
            "invalid rule",
            "empty request body",
            Some(1),
        ));
    }

    match serde_json::from_str::<RuleDef>(text) {
        Ok(rule) => Ok(rule),
        Err(json_err) => {
            if let Some(line) = contains_yaml_anchor_or_alias(text) {
                return Err(err_with_line(
                    StatusCode::BAD_REQUEST,
                    "invalid rule",
                    "YAML anchors and aliases are not allowed",
                    Some(line),
                ));
            }
            match serde_yaml::from_str::<RuleDef>(text) {
                Ok(rule) => Ok(rule),
                Err(yaml_err) => {
                    let line = yaml_err
                        .location()
                        .map(|l| l.line())
                        .or_else(|| Some(json_err.line()));
                    Err(err_with_line(
                        StatusCode::BAD_REQUEST,
                        "invalid rule",
                        yaml_err,
                        line,
                    ))
                }
            }
        }
    }
}

/// Semantically validates a rule: required fields, condition compilation, and presentation templates.
pub fn validate_rule(rule: &RuleDef) -> ApiResult<()> {
    if rule.id.trim().is_empty() {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "invalid rule",
            "rule id cannot be empty",
        ));
    }
    if rule.name.trim().is_empty() {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "invalid rule",
            "rule name cannot be empty",
        ));
    }
    if rule.recipients.is_empty() {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "invalid rule",
            "rule must have at least one recipient",
        ));
    }
    for r in &rule.recipients {
        let id = match r {
            contracts::Recipient::Person(id) | contracts::Recipient::Team(id) => id,
        };
        if id.trim().is_empty() {
            return Err(err(
                StatusCode::BAD_REQUEST,
                "invalid rule",
                "recipient id cannot be empty",
            ));
        }
    }

    condition::compile(&rule.condition)
        .map_err(|e| err(StatusCode::BAD_REQUEST, "invalid rule condition", e))?;

    if let Some(title) = &rule.presentation.title {
        condition::validate_template(title)
            .map_err(|e| err(StatusCode::BAD_REQUEST, "invalid title template", e))?;
    }
    if let Some(body) = &rule.presentation.body {
        condition::validate_template(body)
            .map_err(|e| err(StatusCode::BAD_REQUEST, "invalid body template", e))?;
    }

    Ok(())
}

fn rule_from_row(r: &sqlx::postgres::PgRow) -> ApiResult<RuleDef> {
    let mut def: RuleDef = serde_json::from_value(opt_json(r, "def"))
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, "invalid stored rule", e))?;
    def.version = r.get::<i64, _>("version");
    def.enabled = r.get::<bool, _>("enabled");
    Ok(def)
}

async fn upsert_rule_in_db(db: &PgPool, mut rule: RuleDef) -> ApiResult<RuleDef> {
    let mut tx = db.begin().await.map_err(db_err)?;
    let initial_stored = serde_json::to_value(&rule)
        .map_err(|e| err(StatusCode::BAD_REQUEST, "invalid rule", e))?;
    let version: i64 = sqlx::query(
        "INSERT INTO rules (id, name, team, def, enabled, version, updated_at) \
         VALUES ($1, $2, $3, $4, $5, 1, now()) \
         ON CONFLICT (id) DO UPDATE SET \
             name = EXCLUDED.name, \
             team = EXCLUDED.team, \
             def = EXCLUDED.def, \
             enabled = EXCLUDED.enabled, \
             version = rules.version + 1, \
             updated_at = now() \
         RETURNING version::bigint AS version",
    )
    .bind(&rule.id)
    .bind(&rule.name)
    .bind(&rule.team)
    .bind(SqlJson(initial_stored))
    .bind(rule.enabled)
    .fetch_one(&mut *tx)
    .await
    .map_err(db_err)?
    .get("version");

    rule.version = version;
    let updated_stored = serde_json::to_value(&rule)
        .map_err(|e| err(StatusCode::BAD_REQUEST, "invalid rule", e))?;
    sqlx::query("UPDATE rules SET def = $1 WHERE id = $2")
        .bind(SqlJson(updated_stored))
        .bind(&rule.id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

    sqlx::query("SELECT pg_notify('config_changed', 'rules')")
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    Ok(rule)
}

async fn list_rules(State(st): State<AppState>) -> ApiResult<Json<Vec<RuleDef>>> {
    ensure_rules_seeded(&st.db, &st.config.config_dir).await;
    let rows = sqlx::query("SELECT def, version::bigint AS version, enabled FROM rules ORDER BY id")
        .fetch_all(&st.db)
        .await
        .map_err(db_err)?;
    rows.iter()
        .map(rule_from_row)
        .collect::<ApiResult<Vec<_>>>()
        .map(Json)
}

async fn create_rule(
    State(st): State<AppState>,
    body: Bytes,
) -> ApiResult<(StatusCode, Json<RuleDef>)> {
    ensure_rules_seeded(&st.db, &st.config.config_dir).await;
    let rule = parse_rule_payload(&body)?;
    validate_rule(&rule)?;
    let saved = upsert_rule_in_db(&st.db, rule).await?;
    Ok((StatusCode::CREATED, Json(saved)))
}

async fn get_rule(State(st): State<AppState>, Path(rule_id): Path<String>) -> ApiResult<Json<RuleDef>> {
    ensure_rules_seeded(&st.db, &st.config.config_dir).await;
    let r = sqlx::query("SELECT def, version::bigint AS version, enabled FROM rules WHERE id = $1")
        .bind(&rule_id)
        .fetch_optional(&st.db)
        .await
        .map_err(db_err)?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "rule not found", &rule_id))?;
    rule_from_row(&r).map(Json)
}

async fn put_rule(
    State(st): State<AppState>,
    Path(rule_id): Path<String>,
    body: Bytes,
) -> ApiResult<Json<RuleDef>> {
    ensure_rules_seeded(&st.db, &st.config.config_dir).await;
    let rule = parse_rule_payload(&body)?;
    if rule.id != rule_id {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "rule_id mismatch",
            "id w body różni się od ścieżki",
        ));
    }
    validate_rule(&rule)?;
    let saved = upsert_rule_in_db(&st.db, rule).await?;
    Ok(Json(saved))
}

async fn delete_rule(
    State(st): State<AppState>,
    Path(rule_id): Path<String>,
) -> ApiResult<StatusCode> {
    ensure_rules_seeded(&st.db, &st.config.config_dir).await;
    let mut tx = st.db.begin().await.map_err(db_err)?;
    sqlx::query("DELETE FROM rules WHERE id = $1")
        .bind(&rule_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    sqlx::query("SELECT pg_notify('config_changed', 'rules')")
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    Ok(StatusCode::NO_CONTENT)
}

// ------------------------------------------------------------------ POST /rules/draft

#[derive(Deserialize)]
struct RuleDraftApiRequest {
    prompt: String,
}

async fn draft_rule(
    State(st): State<AppState>,
    body: Result<Json<RuleDraftApiRequest>, JsonRejection>,
) -> ApiResult<Json<runbooks::author::RuleDraftOutcome>> {
    if !st.author.is_enabled() {
        return Err(err(
            StatusCode::NOT_IMPLEMENTED,
            "llm author disabled",
            "Asystent AI jest wyłączony (ustaw LLM_PROVIDER=local)",
        ));
    }
    let Json(req) =
        body.map_err(|e| err(StatusCode::BAD_REQUEST, "invalid draft request", e.body_text()))?;
    if req.prompt.trim().is_empty() {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "invalid draft request",
            "opis reguły nie może być pusty",
        ));
    }

    ensure_rules_seeded(&st.db, &st.config.config_dir).await;
    crate::routes::people::ensure_people_seeded(&st.db, &st.config.config_dir).await;

    let team_rows = sqlx::query("SELECT id, name FROM teams ORDER BY id")
        .fetch_all(&st.db)
        .await
        .map_err(db_err)?;
    let available_teams: Vec<(String, String)> = team_rows
        .iter()
        .map(|r| (r.get::<String, _>("id"), r.get::<String, _>("name")))
        .collect();

    let people_rows = sqlx::query("SELECT id, name FROM people ORDER BY id")
        .fetch_all(&st.db)
        .await
        .map_err(db_err)?;
    let available_people: Vec<(String, String)> = people_rows
        .iter()
        .map(|r| (r.get::<String, _>("id"), r.get::<String, _>("name")))
        .collect();

    let rule_rows = sqlx::query("SELECT def, version::bigint AS version, enabled FROM rules ORDER BY id")
        .fetch_all(&st.db)
        .await
        .map_err(db_err)?;
    let existing_rules: Vec<RuleDef> = rule_rows
        .iter()
        .filter_map(|r| rule_from_row(r).ok())
        .collect();

    let action_rows: Vec<(Value, i64)> = sqlx::query_as("SELECT definition, version FROM actions ORDER BY name")
        .fetch_all(&st.db)
        .await
        .map_err(db_err)?;
    let mut available_actions: Vec<contracts::ActionDef> = action_rows
        .into_iter()
        .filter_map(|(d, v)| serde_json::from_value::<contracts::ActionDef>(d).ok().map(|a| contracts::ActionDef { version: v, ..a }))
        .collect();
    for b in runbooks::registry::Registry::builtins() {
        if !available_actions.iter().any(|a| a.name == b.name) {
            available_actions.push(b);
        }
    }

    let mut available_fields: Vec<String> = vec![
        "schema",
        "severity",
        "source",
        "title",
        "service",
        "raw.type",
        "raw.error.code",
        "raw.workflowName",
        "raw.alertname",
        "raw.labels.service",
        "raw.labels.region",
        "raw.payload.desk",
        "raw.payload.reason",
        "raw.payload.notional",
        "raw.payload.currency",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();

    for schema_name in ["trade.rejected.v2", "workflow.v1", "alertmanager"] {
        for &f in builtin_schema_fields(schema_name) {
            if !available_fields.iter().any(|x| x == f) {
                available_fields.push(f.to_string());
            }
        }
    }

    let draft_req = runbooks::author::RuleDraftRequest {
        prompt: req.prompt,
        available_teams,
        available_people,
        available_fields,
        existing_rules,
        available_actions,
    };

    let outcome = st.author.draft_rule(&draft_req).await.map_err(|e| match e {
        runbooks::author::AuthorError::Disabled => {
            err(StatusCode::NOT_IMPLEMENTED, "llm author disabled", e)
        }
        runbooks::author::AuthorError::InvalidInput(msg) => {
            err(StatusCode::BAD_REQUEST, "invalid draft request", msg)
        }
        runbooks::author::AuthorError::Timeout(_) => {
            err(StatusCode::GATEWAY_TIMEOUT, "llm timeout", e)
        }
        runbooks::author::AuthorError::Upstream(_)
        | runbooks::author::AuthorError::InvalidResponse(_) => {
            err(StatusCode::BAD_GATEWAY, "llm error", e)
        }
    })?;

    Ok(Json(outcome))
}

// ------------------------------------------------------------------ POST /rules/test

#[derive(Deserialize)]
struct RuleTestRequest {
    rule: RuleDef,
    #[serde(default)]
    event: Option<Value>,
    #[serde(default)]
    event_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleTestResult {
    pub matched: bool,
    pub explanation: contracts::Explanation,
    pub rendered: Rendered,
}

fn parse_severity_fuzzy(val: Option<&Value>) -> Severity {
    match val.and_then(Value::as_str).map(str::to_ascii_lowercase).as_deref() {
        Some("critical" | "crit" | "fatal") => Severity::Critical,
        Some("error" | "err") => Severity::Error,
        Some("warning" | "warn") => Severity::Warning,
        _ => Severity::Info,
    }
}

/// Coerces an arbitrary event JSON (full `CanonicalEvent`, `IncomingEnvelope`, or raw payload)
/// into a `CanonicalEvent`.
pub fn coerce_to_canonical_event(val: Value) -> ApiResult<CanonicalEvent> {
    if let Ok(ev) = serde_json::from_value::<CanonicalEvent>(val.clone()) {
        return Ok(ev);
    }

    let Value::Object(map) = val else {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "invalid event",
            "event must be a JSON object",
        ));
    };

    if map.contains_key("raw") {
        let event_id = map
            .get("event_id")
            .and_then(Value::as_str)
            .and_then(|s| Uuid::parse_str(s).ok())
            .unwrap_or_else(Uuid::nil);
        let raw = map.get("raw").cloned().unwrap_or(Value::Null);
        let schema = map
            .get("schema")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .or_else(|| raw.pointer("/meta/schema").and_then(Value::as_str))
            .unwrap_or("")
            .to_string();
        let source = map
            .get("source")
            .and_then(Value::as_str)
            .unwrap_or("test")
            .to_string();
        let severity = parse_severity_fuzzy(map.get("severity").or_else(|| raw.get("severity")));
        let title = map
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or(&schema)
            .to_string();
        let fingerprint = map
            .get("fingerprint")
            .and_then(Value::as_str)
            .unwrap_or("test")
            .to_string();
        let resolved = map
            .get("resolved")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let fields = map
            .get("fields")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let received_at = map
            .get("received_at")
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<DateTime<Utc>>().ok())
            .unwrap_or_else(Utc::now);
        let mapping_version = map
            .get("mapping_version")
            .and_then(Value::as_i64)
            .unwrap_or(0);

        return Ok(CanonicalEvent {
            event_id,
            schema,
            source,
            severity,
            title,
            fingerprint,
            resolved,
            fields,
            raw,
            received_at,
            mapping_version,
        });
    }

    if map.contains_key("format") && map.contains_key("schema") && map.contains_key("payload") {
        let schema = map
            .get("schema")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let raw = map.get("payload").cloned().unwrap_or(Value::Null);
        let severity = parse_severity_fuzzy(
            raw.get("severity")
                .or_else(|| raw.pointer("/labels/severity")),
        );
        return Ok(CanonicalEvent {
            event_id: Uuid::nil(),
            schema: schema.clone(),
            source: "test".into(),
            severity,
            title: schema,
            fingerprint: "test".into(),
            resolved: false,
            fields: Map::new(),
            raw,
            received_at: Utc::now(),
            mapping_version: 0,
        });
    }

    let raw = Value::Object(map);
    let schema = raw
        .get("schema")
        .and_then(Value::as_str)
        .or_else(|| raw.pointer("/meta/schema").and_then(Value::as_str))
        .unwrap_or("")
        .to_string();
    let severity = parse_severity_fuzzy(
        raw.get("severity")
            .or_else(|| raw.pointer("/labels/severity")),
    );
    Ok(CanonicalEvent {
        event_id: Uuid::nil(),
        schema: schema.clone(),
        source: "test".into(),
        severity,
        title: schema,
        fingerprint: "test".into(),
        resolved: false,
        fields: Map::new(),
        raw,
        received_at: Utc::now(),
        mapping_version: 0,
    })
}

pub fn evaluate_rule_on_event(rule: &RuleDef, event: &CanonicalEvent) -> ApiResult<RuleTestResult> {
    let compiled = condition::compile(&rule.condition)
        .map_err(|e| err(StatusCode::BAD_REQUEST, "invalid rule condition", e))?;
    if let Some(t) = &rule.presentation.title {
        condition::validate_template(t)
            .map_err(|e| err(StatusCode::BAD_REQUEST, "invalid title template", e))?;
    }
    if let Some(b) = &rule.presentation.body {
        condition::validate_template(b)
            .map_err(|e| err(StatusCode::BAD_REQUEST, "invalid body template", e))?;
    }

    let ctx = condition::event_context(event);
    let matched = compiled.evaluate(&ctx);
    let explanation = compiled.explain(&ctx);

    let title = match &rule.presentation.title {
        Some(tpl) if !tpl.is_empty() => condition::render_template(tpl, &ctx)
            .map_err(|e| err(StatusCode::BAD_REQUEST, "invalid title template", e))?,
        _ => event.title.clone(),
    };
    let body = match &rule.presentation.body {
        Some(tpl) if !tpl.is_empty() => condition::render_template(tpl, &ctx)
            .map_err(|e| err(StatusCode::BAD_REQUEST, "invalid body template", e))?,
        _ => String::new(),
    };

    Ok(RuleTestResult {
        matched,
        explanation,
        rendered: Rendered {
            title,
            body,
            link: None,
        },
    })
}

async fn lookup_event_by_id(db: &PgPool, event_id: Uuid) -> ApiResult<CanonicalEvent> {
    let event_id_str = event_id.to_string();
    if let Some(row) = sqlx::query(
        "SELECT last_event FROM alerts WHERE (last_event->>'event_id') = $1 OR id = $2 \
         ORDER BY last_seen DESC LIMIT 1",
    )
    .bind(&event_id_str)
    .bind(event_id)
    .fetch_optional(db)
    .await
    .map_err(db_err)?
    {
        return coerce_to_canonical_event(opt_json(&row, "last_event"));
    }

    if let Some(row) = sqlx::query(
        "SELECT schema, envelope FROM unmapped_events WHERE event_id = $1 LIMIT 1",
    )
    .bind(event_id)
    .fetch_optional(db)
    .await
    .map_err(db_err)?
    {
        let schema: String = row.get("schema");
        let env = opt_json(&row, "envelope");
        let payload = env.get("payload").cloned().unwrap_or(env);
        return coerce_to_canonical_event(json!({
            "event_id": event_id,
            "schema": schema,
            "raw": payload,
        }));
    }

    Err(err(
        StatusCode::BAD_REQUEST,
        "event not found",
        format!("no event found with event_id {event_id}"),
    ))
}

async fn test_rule(
    State(st): State<AppState>,
    body: Result<Json<RuleTestRequest>, JsonRejection>,
) -> ApiResult<Json<RuleTestResult>> {
    let Json(req) =
        body.map_err(|e| err(StatusCode::BAD_REQUEST, "invalid test request", e.body_text()))?;

    let canonical_event = if let Some(ev_val) = req.event {
        coerce_to_canonical_event(ev_val)?
    } else if let Some(eid) = req.event_id {
        lookup_event_by_id(&st.db, eid).await?
    } else {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "missing event",
            "provide either 'event' or 'event_id'",
        ));
    };

    let res = evaluate_rule_on_event(&req.rule, &canonical_event)?;
    Ok(Json(res))
}

// ------------------------------------------------------------------ GET /fields?schema=...

#[derive(Deserialize)]
struct FieldsQuery {
    schema: String,
}

fn collect_leaf_paths(prefix: &str, val: &Value, out: &mut Vec<String>, seen: &mut BTreeSet<String>) {
    match val {
        Value::Object(map) => {
            for (k, v) in map {
                let next = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                if v.is_object() {
                    collect_leaf_paths(&next, v, out, seen);
                } else if seen.insert(next.clone()) {
                    out.push(next);
                }
            }
        }
        _ => {
            if !prefix.is_empty() && seen.insert(prefix.to_string()) {
                out.push(prefix.to_string());
            }
        }
    }
}

fn jsonpath_to_raw_field(jp: &str) -> Option<String> {
    let trimmed = jp.trim();
    let stripped = trimmed.strip_prefix("$.")?;
    if stripped.is_empty() {
        return None;
    }
    Some(format!("raw.{stripped}"))
}

fn builtin_schema_fields(schema: &str) -> &'static [&'static str] {
    match schema {
        "trade.rejected.v2" => &[
            "desk",
            "tradeId",
            "raw.meta.schema",
            "raw.meta.producer",
            "raw.meta.ts",
            "raw.payload.tradeId",
            "raw.payload.desk",
            "raw.payload.notional",
            "raw.payload.currency",
            "raw.payload.reason",
        ],
        "workflow.v1" => &[
            "type",
            "workflowName",
            "workflowId",
            "step",
            "environment",
            "raw.type",
            "raw.workflowId",
            "raw.workflowName",
            "raw.step",
            "raw.severity",
            "raw.environment",
            "raw.timestamp",
            "raw.error.code",
            "raw.error.message",
        ],
        "alertmanager" => &[
            "service",
            "region",
            "raw.alertname",
            "raw.labels.service",
            "raw.labels.region",
            "raw.labels.severity",
            "raw.annotations.summary",
            "raw.startsAt",
            "raw.status",
        ],
        _ => &[],
    }
}

pub fn suggest_fields_with_samples(
    schema: &str,
    mapping_defs: &[MappingDef],
    canonical_events: &[Value],
    raw_payloads: &[Value],
) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();

    for c in ["schema", "severity", "source", "title", "fingerprint", "resolved"] {
        if seen.insert(c.to_string()) {
            out.push(c.to_string());
        }
    }

    for m in mapping_defs {
        for (field_name, jp) in &m.fields {
            if seen.insert(field_name.clone()) {
                out.push(field_name.clone());
            }
            if let Some(raw_path) = jsonpath_to_raw_field(jp) {
                if seen.insert(raw_path.clone()) {
                    out.push(raw_path);
                }
            }
        }
        for fp in &m.fingerprint {
            if let Some(raw_path) = jsonpath_to_raw_field(fp) {
                if seen.insert(raw_path.clone()) {
                    out.push(raw_path);
                }
            }
        }
        if let Some(title) = &m.title {
            if let Some(raw_path) = jsonpath_to_raw_field(title) {
                if seen.insert(raw_path.clone()) {
                    out.push(raw_path);
                }
            }
        }
    }

    for ev in canonical_events {
        if let Some(fields) = ev.get("fields") {
            collect_leaf_paths("", fields, &mut out, &mut seen);
        }
        if let Some(raw) = ev.get("raw") {
            collect_leaf_paths("raw", raw, &mut out, &mut seen);
        }
    }

    for payload in raw_payloads {
        collect_leaf_paths("raw", payload, &mut out, &mut seen);
    }

    for &f in builtin_schema_fields(schema) {
        if seen.insert(f.to_string()) {
            out.push(f.to_string());
        }
    }

    out
}

fn validate_schema_name(schema: &str) -> Result<(), &'static str> {
    if schema.is_empty() {
        return Err("schema cannot be empty");
    }
    if schema == "."
        || schema.contains("..")
        || schema.contains('/')
        || schema.contains('\\')
        || schema.contains('\0')
    {
        return Err("schema contains forbidden path characters");
    }
    if !schema
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err("schema may only contain [A-Za-z0-9._-]");
    }
    Ok(())
}

async fn get_fields(
    State(st): State<AppState>,
    Query(q): Query<FieldsQuery>,
) -> ApiResult<Json<Vec<String>>> {
    let schema = q.schema.trim();
    validate_schema_name(schema)
        .map_err(|msg| err(StatusCode::BAD_REQUEST, "invalid schema", msg))?;

    let mut mapping_defs = Vec::new();
    if let Ok(Some(row)) = sqlx::query("SELECT def FROM mappings WHERE schema = $1")
        .bind(schema)
        .fetch_optional(&st.db)
        .await
    {
        if let Ok(m) = serde_json::from_value::<MappingDef>(opt_json(&row, "def")) {
            mapping_defs.push(m);
        }
    }
    if mapping_defs.is_empty() {
        if let Some(mapping_file) =
            resolve_config_path(&st.config.config_dir, &format!("mappings/{schema}.yaml"))
        {
            if let Ok(text) = std::fs::read_to_string(mapping_file) {
                if let Ok(m) = serde_yaml::from_str::<MappingDef>(&text) {
                    mapping_defs.push(m);
                }
            }
        }
    }

    let mut canonical_events = Vec::new();
    if let Ok(rows) = sqlx::query(
        "SELECT last_event FROM alerts WHERE schema = $1 ORDER BY last_seen DESC LIMIT 20",
    )
    .bind(schema)
    .fetch_all(&st.db)
    .await
    {
        for r in rows {
            canonical_events.push(opt_json(&r, "last_event"));
        }
    }

    let mut raw_payloads = Vec::new();
    if let Ok(rows) = sqlx::query(
        "SELECT envelope->'payload' AS payload FROM unmapped_events WHERE schema = $1 \
         ORDER BY received_at DESC LIMIT 10",
    )
    .bind(schema)
    .fetch_all(&st.db)
    .await
    {
        for r in rows {
            raw_payloads.push(opt_json(&r, "payload"));
        }
    }

    let out = suggest_fields_with_samples(schema, &mapping_defs, &canonical_events, &raw_payloads);
    Ok(Json(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::people::PeopleConfigFile;
    use contracts::{Condition, Op, Presentation, Recipient};

    #[test]
    fn unit_parse_rule_payload_yaml_and_json_and_error_line() {
        let valid_yaml = r#"
id: rates-big-rejections
name: "Rates: large rejected trades"
team: rates
condition:
  all:
    - { field: schema, op: glob, value: "trade.rejected.*" }
    - { field: raw.payload.desk, op: eq, value: rates }
    - { field: raw.payload.notional, op: gt, value: 10000000 }
recipients:
  - { type: team, id: rates }
presentation:
  title: "Rejected {{raw.payload.currency}} {{raw.payload.notional}}"
"#;
        let rule = parse_rule_payload(valid_yaml.as_bytes()).expect("valid yaml rule");
        assert_eq!(rule.id, "rates-big-rejections");
        assert!(validate_rule(&rule).is_ok());

        let valid_json = serde_json::to_vec(&rule).unwrap();
        let rule_from_json = parse_rule_payload(&valid_json).expect("valid json rule");
        assert_eq!(rule_from_json.id, "rates-big-rejections");

        // Invalid YAML syntax on a known line
        let bad_yaml = "id: bad-rule\nname: Bad\ncondition:\n  field: x\n  op: [unclosed\n";
        let (status, Json(err_val)) = parse_rule_payload(bad_yaml.as_bytes()).unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(err_val["error"], "invalid rule");
        assert!(
            err_val["line"].as_u64().unwrap_or(0) >= 4,
            "expected line number in error: {err_val}"
        );
    }

    #[test]
    fn unit_validate_rule_rejects_bad_regex_and_unsafe_template() {
        let bad_regex_rule = RuleDef {
            id: "r1".into(),
            name: "Bad regex".into(),
            team: None,
            condition: Condition::Leaf {
                field: "service".into(),
                op: Op::Regex,
                value: json!("(unclosed"),
            },
            recipients: vec![Recipient::Person("anna".into())],
            severity_override: None,
            presentation: Presentation::default(),
            enabled: true,
            version: 1,
        };
        let (st, Json(v)) = validate_rule(&bad_regex_rule).unwrap_err();
        assert_eq!(st, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"], "invalid rule condition");

        let bad_tpl_rule = RuleDef {
            id: "r2".into(),
            name: "Bad template".into(),
            team: None,
            condition: Condition::Leaf {
                field: "service".into(),
                op: Op::Eq,
                value: json!("payments-api"),
            },
            recipients: vec![Recipient::Person("anna".into())],
            severity_override: None,
            presentation: Presentation {
                title: Some("Hello {{unclosed".into()),
                ..Default::default()
            },
            enabled: true,
            version: 1,
        };
        let (st, Json(v)) = validate_rule(&bad_tpl_rule).unwrap_err();
        assert_eq!(st, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"], "invalid title template");
    }

    #[test]
    fn unit_evaluate_rule_on_event_explains_and_renders() {
        let rule: RuleDef = serde_yaml::from_str(
            r#"
id: rates-big-rejections
name: "Rates: large rejected trades"
team: rates
condition:
  all:
    - { field: schema, op: glob, value: "trade.rejected.*" }
    - { field: raw.payload.desk, op: eq, value: rates }
    - { field: raw.payload.notional, op: gt, value: 10000000 }
recipients:
  - { type: team, id: rates }
presentation:
  title: "Rejected {{raw.payload.currency}} {{raw.payload.notional}} — {{raw.payload.reason}}"
"#,
        )
        .unwrap();

        let ev = coerce_to_canonical_event(json!({
            "format": "json",
            "schema": "trade.rejected.v2",
            "payload": {
                "meta": { "schema": "trade.rejected.v2", "producer": "trade-gateway" },
                "payload": {
                    "tradeId": "T-998877",
                    "desk": "rates",
                    "notional": 5000000,
                    "currency": "EUR",
                    "reason": "LIMIT_BREACH"
                }
            }
        }))
        .unwrap();

        let res = evaluate_rule_on_event(&rule, &ev).unwrap();
        assert!(!res.matched);
        assert_eq!(res.explanation.node, "all");
        assert_eq!(res.explanation.children.len(), 3);
        assert!(res.explanation.children[0].result);
        assert!(res.explanation.children[1].result);
        assert!(!res.explanation.children[2].result);
        assert_eq!(res.explanation.children[2].actual, Some(json!(5000000)));
        assert_eq!(res.rendered.title, "Rejected EUR 5000000 — LIMIT_BREACH");
    }

    #[test]
    fn unit_suggest_fields_for_trade_rejected_v2() {
        let fields = suggest_fields_with_samples("trade.rejected.v2", &[], &[], &[]);
        assert!(fields.contains(&"schema".to_string()));
        assert!(fields.contains(&"severity".to_string()));
        assert!(fields.contains(&"raw.payload.notional".to_string()));
        assert!(fields.contains(&"raw.payload.desk".to_string()));
        assert!(fields.contains(&"raw.payload.tradeId".to_string()));
        assert!(fields.contains(&"raw.payload.currency".to_string()));
        assert!(fields.contains(&"raw.payload.reason".to_string()));

        // Also test dynamic extraction from CanonicalEvent and raw unmapped payload
        let custom_fields = suggest_fields_with_samples(
            "custom.schema.v1",
            &[],
            &[json!({
                "fields": { "customTop": "val" },
                "raw": { "nested": { "deepLeaf": 123 } }
            })],
            &[json!({ "unmappedLeaf": true })],
        );
        assert!(custom_fields.contains(&"customTop".to_string()));
        assert!(custom_fields.contains(&"raw.nested.deepLeaf".to_string()));
        assert!(custom_fields.contains(&"raw.unmappedLeaf".to_string()));
        assert!(!custom_fields.contains(&"raw.nested".to_string()));
    }

    #[test]
    fn unit_seed_files_are_valid() {
        let rules_dir = resolve_config_path("config", "rules").expect("resolve config/rules");
        let entries: Vec<_> = std::fs::read_dir(&rules_dir)
            .expect("read config/rules")
            .filter_map(|e| e.ok().map(|x| x.path()))
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("yaml"))
            .collect();
        assert!(entries.len() >= 4, "expected at least 4 seed rule files");
        for path in entries {
            let bytes = std::fs::read(&path).unwrap();
            let rule = parse_rule_payload(&bytes).unwrap_or_else(|e| panic!("invalid {}: {:?}", path.display(), e));
            validate_rule(&rule).unwrap_or_else(|e| panic!("invalid rule {}: {:?}", path.display(), e));
        }

        let people_path = resolve_config_path("config", "people.yaml").expect("resolve config/people.yaml");
        let text = std::fs::read_to_string(&people_path).expect("read config/people.yaml");
        let parsed: PeopleConfigFile = serde_yaml::from_str(&text).expect("valid people.yaml");
        assert_eq!(parsed.people.len(), 6);
        assert_eq!(parsed.teams.len(), 5);
    }

    #[test]
    fn unit_schema_validation_and_path_traversal_rejected() {
        assert!(validate_schema_name("trade.rejected.v2").is_ok());
        assert!(validate_schema_name("workflow_v1-beta").is_ok());
        for bad in ["", ".", "..", "../people", "a/b", "a\\b", "a..b", "foo\0bar", "foo bar", "foo$bar"] {
            assert!(validate_schema_name(bad).is_err(), "should reject schema `{bad}`");
        }
        assert!(resolve_config_path("config", "../Cargo.toml").is_none());
        assert!(resolve_config_path("config", "/etc/passwd").is_none());
        assert!(resolve_config_path("config", "mappings/../../Cargo.toml").is_none());
        assert!(resolve_config_path("config", "people.yaml\0").is_none());
    }

    #[test]
    fn unit_yaml_anchors_and_aliases_rejected() {
        let bomb = "a: &a [\"lol\", \"lol\"]\nb: [*a, *a]\n";
        let (st, Json(v)) = parse_rule_payload(bomb.as_bytes()).unwrap_err();
        assert_eq!(st, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"], "invalid rule");
        assert!(v["detail"].as_str().unwrap_or("").contains("anchors"));
    }
}
