//! Shared rule loading, team matching, and alert presentation enrichment.

use common::db::sqlx::{self, PgPool, Row};
use contracts::{CanonicalEvent, Recipient, RuleDef, Severity};
use serde_json::{json, Value};

pub struct CompiledRule {
    pub def: RuleDef,
    pub condition: condition::Compiled,
    pub teams: Vec<String>,
}

impl CompiledRule {
    pub fn from_def(def: RuleDef) -> Option<Self> {
        if !def.enabled {
            return None;
        }
        let condition = condition::compile(&def.condition).ok()?;
        let mut teams: Vec<String> = def.team.iter().cloned().collect();
        for r in &def.recipients {
            if let Recipient::Team(t) = r {
                if !teams.contains(t) {
                    teams.push(t.clone());
                }
            }
        }
        Some(Self { def, condition, teams })
    }
}

pub async fn load_enabled_rules(db: &PgPool) -> Vec<CompiledRule> {
    match sqlx::query("SELECT def, enabled FROM rules WHERE enabled = TRUE ORDER BY id")
        .fetch_all(db)
        .await
    {
        Ok(rows) => rows
            .iter()
            .filter_map(|row| {
                let mut def: RuleDef = serde_json::from_value(row.get("def")).ok()?;
                def.enabled = row.get("enabled");
                CompiledRule::from_def(def)
            })
            .collect(),
        Err(e) => {
            tracing::warn!(error = %e, "loading rules failed, returning empty rule set");
            Vec::new()
        }
    }
}

/// Teams associated with an event = union of teams from all matching enabled rules.
pub fn teams_for_event(rules: &[CompiledRule], event: &CanonicalEvent) -> Vec<String> {
    let ctx = condition::event_context(event);
    let mut out: Vec<String> = Vec::new();
    for r in rules.iter().filter(|r| r.condition.evaluate(&ctx)) {
        for t in &r.teams {
            if !out.contains(t) {
                out.push(t.clone());
            }
        }
    }
    out
}

fn severity_to_str(s: Severity) -> &'static str {
    match s {
        Severity::Info => "info",
        Severity::Warning => "warning",
        Severity::Error => "error",
        Severity::Critical => "critical",
    }
}

fn resolve_path(ctx: &Value, path: &str) -> Value {
    let mut cur = ctx;
    for seg in path.split('.') {
        match cur {
            Value::Object(map) => match map.get(seg) {
                Some(next) => cur = next,
                None => return Value::Null,
            },
            Value::Array(arr) => match seg.parse::<usize>().ok().and_then(|i| arr.get(i)) {
                Some(next) => cur = next,
                None => return Value::Null,
            },
            _ => return Value::Null,
        }
    }
    cur.clone()
}

pub struct AlertEnrichment {
    pub rule_ids: Vec<String>,
    pub teams: Vec<String>,
    pub severity: String,
    pub title: String,
    pub color: Option<String>,
    pub body: String,
    pub fields: Vec<Value>,
}

/// Computes `rule_ids`, `teams`, overridden `severity`, and presentation (`title`, `body`, `fields`, `color`)
/// from enabled rules.
pub fn enrich_alert(base_severity: &str, base_title: &str, last_event: &Value, rules: &[CompiledRule]) -> AlertEnrichment {
    let mut out = AlertEnrichment {
        rule_ids: Vec::new(),
        teams: Vec::new(),
        severity: base_severity.to_string(),
        title: base_title.to_string(),
        color: None,
        body: String::new(),
        fields: Vec::new(),
    };
    let Ok(event) = serde_json::from_value::<CanonicalEvent>(last_event.clone()) else {
        return out;
    };
    let ctx = condition::event_context(&event);
    let mut max_override: Option<Severity> = None;
    let mut title_set = false;
    let mut body_set = false;
    let mut seen_fields: Vec<String> = Vec::new();

    for rule in rules {
        if !rule.condition.evaluate(&ctx) {
            continue;
        }
        out.rule_ids.push(rule.def.id.clone());
        for t in &rule.teams {
            if !out.teams.contains(t) {
                out.teams.push(t.clone());
            }
        }
        if let Some(ov) = rule.def.severity_override {
            max_override = Some(match max_override {
                Some(cur) => cur.max(ov),
                None => ov,
            });
        }
        if !title_set {
            if let Some(tpl) = rule.def.presentation.title.as_deref().filter(|s| !s.trim().is_empty()) {
                if let Ok(rendered) = condition::render_template(tpl, &ctx) {
                    if !rendered.trim().is_empty() {
                        out.title = rendered;
                        title_set = true;
                    }
                }
            }
        }
        if out.color.is_none() {
            if let Some(c) = rule.def.presentation.color.as_deref().filter(|s| !s.trim().is_empty()) {
                out.color = Some(c.to_string());
            }
        }
        if !body_set {
            if let Some(tpl) = rule.def.presentation.body.as_deref().filter(|s| !s.trim().is_empty()) {
                if let Ok(rendered) = condition::render_template(tpl, &ctx) {
                    out.body = rendered;
                    body_set = true;
                }
            }
        }
        for path in &rule.def.presentation.fields {
            if seen_fields.contains(path) {
                continue;
            }
            seen_fields.push(path.clone());
            let label = path.strip_prefix("raw.").unwrap_or(path);
            let value = resolve_path(&ctx, path);
            let highlight = rule.def.presentation.highlight.get(path).cloned();
            out.fields.push(json!({
                "path": path,
                "label": label,
                "value": value,
                "highlight": highlight,
            }));
        }
    }

    if let Some(ov) = max_override {
        out.severity = severity_to_str(ov).to_string();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_enrich_alert_settlements_presentation_and_override() {
        let rule: RuleDef = serde_json::from_str(include_str!("../../contracts/fixtures/config/rule.settlements.json")).unwrap();
        let compiled = CompiledRule::from_def(rule).unwrap();
        let alert: contracts::AlertChanged =
            serde_json::from_str(include_str!("../../contracts/fixtures/topics/alerts.changed.json")).unwrap();
        let last_event = serde_json::to_value(&alert.event).unwrap();

        let e = enrich_alert("error", &alert.event.title, &last_event, &[compiled]);
        assert_eq!(e.rule_ids, vec!["settlements-prod"]);
        assert_eq!(e.teams, vec!["settlements"]);
        assert_eq!(e.severity, "critical");
        assert_eq!(e.title, "WORKFLOW_FAILED order-settlement");
        assert_eq!(e.color.as_deref(), Some("red"));
        assert_eq!(e.body, "Step validate-counterparty failed: Counterparty service did not respond in 5000 ms");
        assert_eq!(e.fields.len(), 3);
        assert_eq!(e.fields[0]["path"], "raw.workflowId");
        assert_eq!(e.fields[0]["label"], "workflowId");
        assert_eq!(e.fields[0]["value"], "WF-20261023-00042");
        assert_eq!(e.fields[0]["highlight"], Value::Null);
        assert_eq!(e.fields[1]["path"], "raw.step");
        assert_eq!(e.fields[1]["highlight"], "red");
        assert_eq!(e.fields[2]["path"], "raw.error.message");
        assert_eq!(e.fields[2]["highlight"], "red");
    }

    #[test]
    fn unit_enrich_alert_unmatched_and_non_canonical_fallback() {
        let rule: RuleDef = serde_json::from_str(include_str!("../../contracts/fixtures/config/rule.rates.json")).unwrap();
        let compiled = CompiledRule::from_def(rule).unwrap();
        let alert: contracts::AlertChanged =
            serde_json::from_str(include_str!("../../contracts/fixtures/topics/alerts.changed.json")).unwrap();
        let last_event = serde_json::to_value(&alert.event).unwrap();

        let e = enrich_alert("error", "base-title", &last_event, std::slice::from_ref(&compiled));
        assert!(e.rule_ids.is_empty());
        assert!(e.teams.is_empty());
        assert_eq!(e.severity, "error");
        assert_eq!(e.title, "base-title");
        assert!(e.color.is_none());

        // Niepełny JSON (jak w starszych wierszach testowych) nie psuje odpowiedzi.
        let e2 = enrich_alert("warning", "tytul", &json!({"title": "tytul"}), &[compiled]);
        assert!(e2.rule_ids.is_empty());
        assert_eq!(e2.severity, "warning");
        assert_eq!(e2.title, "tytul");
    }
}
