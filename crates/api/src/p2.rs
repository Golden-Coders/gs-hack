//! Routy strumienia P2: reguły (CRUD, test, podpowiedzi pól), osoby i zespoły,
//! dostarczenia, symulowane kanały. Właściciel: P2. Zob. `contracts/openapi.yaml` (tag `p2`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path as FsPath, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use axum::{
    body::Bytes,
    extract::{rejection::JsonRejection, Path, Query, State},
    http::StatusCode,
    routing::{get, post, put},
    Json, Router,
};
use common::db::sqlx::{
    self,
    types::{chrono::DateTime, chrono::Utc, Json as SqlJson, Uuid},
    PgPool, Row,
};
use contracts::{
    topics, CanonicalEvent, Channel, DeliveryRequest, MappingDef, PersonDef, Rendered, RuleDef,
    Severity, TeamDef,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::AppState;

type ApiResult<T> = Result<T, (StatusCode, Json<Value>)>;

static RULES_SEEDED: AtomicBool = AtomicBool::new(false);
static PEOPLE_SEEDED: AtomicBool = AtomicBool::new(false);

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/rules", get(list_rules).post(create_rule))
        .route("/rules/test", post(test_rule))
        .route("/rules/{rule_id}", get(get_rule).put(put_rule).delete(delete_rule))
        .route("/fields", get(get_fields))
        .route("/people", get(list_people))
        .route("/teams", get(list_teams))
        .route("/alerts/{alert_id}/deliveries", get(alert_deliveries))
        .route("/deliveries", get(list_deliveries))
        .route("/deliveries/{delivery_id}/resend", post(resend_delivery))
        .route("/mock/outbox", get(mock_outbox))
        .route("/mock/channels", get(list_mock_channels))
        .route("/mock/channels/{channel}", put(put_mock_channel))
}

fn err(status: StatusCode, error: &str, detail: impl ToString) -> (StatusCode, Json<Value>) {
    (
        status,
        Json(json!({
            "error": error,
            "detail": detail.to_string(),
        })),
    )
}

fn err_with_line(
    status: StatusCode,
    error: &str,
    detail: impl ToString,
    line: Option<usize>,
) -> (StatusCode, Json<Value>) {
    (
        status,
        Json(json!({
            "error": error,
            "detail": detail.to_string(),
            "line": line,
        })),
    )
}

fn db_err(e: sqlx::Error) -> (StatusCode, Json<Value>) {
    tracing::error!(error = %e, "db error");
    err(StatusCode::INTERNAL_SERVER_ERROR, "database error", e)
}

fn parse_uuid(s: &str, label: &str) -> ApiResult<Uuid> {
    Uuid::parse_str(s).map_err(|e| err(StatusCode::BAD_REQUEST, &format!("invalid {label}"), e))
}

fn opt_json(r: &sqlx::postgres::PgRow, col: &str) -> Value {
    r.get::<Option<SqlJson<Value>>, _>(col)
        .map_or(Value::Null, |j| j.0)
}

/// Znajduje ścieżkę pliku/katalogu konfiguracyjnego niezależnie od tego, czy proces
/// wystartował z katalogu głównego repozytorium, czy z `crates/api`.
fn resolve_config_path(config_dir: &str, rel: &str) -> PathBuf {
    let candidates = [
        FsPath::new(config_dir).join(rel),
        FsPath::new("config").join(rel),
        FsPath::new("../../config").join(rel),
    ];
    for c in &candidates {
        if c.exists() {
            return c.clone();
        }
    }
    candidates[0].clone()
}

// ------------------------------------------------------------------ Seeding (leniwym startem, gdy router jeszcze nie załadował plików)

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
    let rules_dir = resolve_config_path(config_dir, "rules");
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

#[derive(Deserialize)]
struct PeopleConfigFile {
    #[serde(default)]
    people: Vec<PersonDef>,
    #[serde(default)]
    teams: Vec<TeamDef>,
}

async fn ensure_people_seeded(db: &PgPool, config_dir: &str) {
    if PEOPLE_SEEDED.swap(true, Ordering::SeqCst) {
        return;
    }
    let people_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM people")
        .fetch_one(db)
        .await
        .unwrap_or(1);
    let teams_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM teams")
        .fetch_one(db)
        .await
        .unwrap_or(1);
    if people_count > 0 && teams_count > 0 {
        return;
    }
    let path = resolve_config_path(config_dir, "people.yaml");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let Ok(parsed) = serde_yaml::from_str::<PeopleConfigFile>(&text) else {
        return;
    };
    for person in &parsed.people {
        let Ok(def_val) = serde_json::to_value(person) else {
            continue;
        };
        let contacts_val = serde_json::to_value(&person.contacts).unwrap_or_else(|_| json!({}));
        let fallback_val = serde_json::to_value(&person.fallback).unwrap_or_else(|_| json!([]));
        let _ = sqlx::query(
            "INSERT INTO people (id, name, timezone, contacts, fallback, def, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, now()) ON CONFLICT (id) DO NOTHING",
        )
        .bind(&person.id)
        .bind(&person.name)
        .bind(&person.timezone)
        .bind(SqlJson(contacts_val))
        .bind(SqlJson(fallback_val))
        .bind(SqlJson(def_val))
        .execute(db)
        .await;
    }
    for team in &parsed.teams {
        let Ok(def_val) = serde_json::to_value(team) else {
            continue;
        };
        let _ = sqlx::query(
            "INSERT INTO teams (id, name, def, updated_at) \
             VALUES ($1, $2, $3, now()) ON CONFLICT (id) DO NOTHING",
        )
        .bind(&team.id)
        .bind(&team.name)
        .bind(SqlJson(def_val))
        .execute(db)
        .await;

        for member in &team.members {
            let _ = sqlx::query(
                "INSERT INTO team_members (team_id, person_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            )
            .bind(&team.id)
            .bind(member)
            .execute(db)
            .await;
        }
    }
}

// ------------------------------------------------------------------ 1. Reguły CRUD (7.2)

/// Parsuje ciało żądania reguły (obsługuje zarówno JSON jak i YAML) oraz zwraca numer linii przy błędzie.
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
        Err(json_err) => match serde_yaml::from_str::<RuleDef>(text) {
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
        },
    }
}

/// Waliduje semantycznie regułę: pola wymagane, kompilację warunku i szablony prezentacji.
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

// ------------------------------------------------------------------ 2. Test reguły na zdarzeniu (POST /rules/test)

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

/// Konwertuje dowolny JSON zdarzenia (pełne `CanonicalEvent`, kopertę `IncomingEnvelope`
/// albo surowy payload wklejony przez operatora) na `CanonicalEvent`.
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

    // Przypadek 1: Obiekt częściowo kanoniczny z polem `raw`
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

    // Przypadek 2: Koperta `IncomingEnvelope` ({ format, schema, payload })
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

    // Przypadek 3: Surowy payload wklejony bezpośrednio
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

// ------------------------------------------------------------------ 3. Podpowiedzi pól (GET /fields?schema=...)

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

async fn get_fields(
    State(st): State<AppState>,
    Query(q): Query<FieldsQuery>,
) -> ApiResult<Json<Vec<String>>> {
    let schema = q.schema.trim();
    if schema.is_empty() {
        return Ok(Json(vec![
            "schema".into(),
            "severity".into(),
            "source".into(),
            "title".into(),
        ]));
    }

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
        let mapping_file = resolve_config_path(&st.config.config_dir, &format!("mappings/{schema}.yaml"));
        if let Ok(text) = std::fs::read_to_string(mapping_file) {
            if let Ok(m) = serde_yaml::from_str::<MappingDef>(&text) {
                mapping_defs.push(m);
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

// ------------------------------------------------------------------ 4. Osoby i zespoły (GET /people, GET /teams)

async fn list_people(State(st): State<AppState>) -> ApiResult<Json<Value>> {
    ensure_people_seeded(&st.db, &st.config.config_dir).await;
    let rows = sqlx::query("SELECT def FROM people ORDER BY id")
        .fetch_all(&st.db)
        .await
        .map_err(db_err)?;
    let mut people = Vec::with_capacity(rows.len());
    for r in &rows {
        let p: PersonDef = serde_json::from_value(opt_json(r, "def"))
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, "invalid stored person", e))?;
        people.push(p);
    }
    Ok(Json(json!({
        "source_file": "config/people.yaml",
        "people": people,
    })))
}

async fn list_teams(State(st): State<AppState>) -> ApiResult<Json<Vec<TeamDef>>> {
    ensure_people_seeded(&st.db, &st.config.config_dir).await;
    let rows = sqlx::query("SELECT def FROM teams ORDER BY id")
        .fetch_all(&st.db)
        .await
        .map_err(db_err)?;
    let mut teams = Vec::with_capacity(rows.len());
    for r in &rows {
        let t: TeamDef = serde_json::from_value(opt_json(r, "def"))
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, "invalid stored team", e))?;
        teams.push(t);
    }
    Ok(Json(teams))
}

// ------------------------------------------------------------------ 5. Dostarczenia i Resend (9.3, 9.4)

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
        "UPDATE deliveries SET status = 'pending', request = $2, updated_at = now() WHERE id = $1",
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

// ------------------------------------------------------------------ 6. Symulowane kanały (9.4)

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

#[cfg(test)]
mod tests {
    use super::*;
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
        let rules_dir = resolve_config_path("config", "rules");
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

        let people_path = resolve_config_path("config", "people.yaml");
        let text = std::fs::read_to_string(&people_path).expect("read config/people.yaml");
        let parsed: PeopleConfigFile = serde_yaml::from_str(&text).expect("valid people.yaml");
        assert_eq!(parsed.people.len(), 6);
        assert_eq!(parsed.teams.len(), 5);
    }
}
