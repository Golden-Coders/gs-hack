//! Dopasowanie runbooków do alertu i podstawianie wartości (zadanie 11.3).

use contracts::{AlertChanged, Condition, RuleDef, RunbookDef};
use serde_json::{json, Value};

/// Kontekst warunków i szablonów: `condition::event_context` + dane alertu.
/// Klucze z najwyższego poziomu oryginału są też dostępne bezpośrednio (`{{workflowId}}`
/// jak w Appendix B), o ile nie zasłania ich pole kanoniczne.
pub fn alert_context(alert: &AlertChanged) -> Value {
    let mut ctx = alert.event.raw.as_object().cloned().unwrap_or_default();
    if let Value::Object(canonical) = condition::event_context(&alert.event) {
        ctx.extend(canonical);
    }
    ctx.insert("alert_id".into(), json!(alert.alert_id));
    ctx.insert("episode".into(), json!(alert.episode));
    ctx.insert("count".into(), json!(alert.count));
    Value::Object(ctx)
}

/// Liczba warunków-liści: im więcej, tym bardziej szczegółowy runbook.
pub fn specificity(c: &Condition) -> i32 {
    match c {
        Condition::All { all: cs } | Condition::Any { any: cs } => cs.iter().map(specificity).sum(),
        Condition::Not { not } => specificity(not),
        Condition::Leaf { .. } => 1,
    }
}

fn evaluates(cond: &Condition, ctx: &Value) -> bool {
    condition::compile(cond).is_ok_and(|c| c.evaluate(ctx))
}

/// Pasujące runbooki od najbardziej szczegółowego (remis: po id, żeby kolejność była stała).
///
/// Runbook pasuje, gdy spełniony jest jego `matches` ALBO gdy pasuje reguła z `rule_ids`
/// (ta sama semantyka co `router::match_manual_runbooks`). Warunki liczymy na tym samym
/// kontekście co router (`condition::event_context`), więc runbook MANUAL przy alercie
/// to ten sam, który router dołącza do powiadomienia. Szczegółowość: liczba warunków
/// w `matches`, a dla runbooka dopasowanego tylko przez regułę – w warunku tej reguły.
pub fn matching<'a>(
    runbooks: &'a [RunbookDef],
    rules: &[RuleDef],
    alert: &AlertChanged,
) -> Vec<(&'a RunbookDef, i32)> {
    let ctx = condition::event_context(&alert.event);
    let pinned_needed = runbooks.iter().any(|rb| !rb.rule_ids.is_empty());
    let matched_rules: Vec<&RuleDef> = if pinned_needed {
        rules
            .iter()
            .filter(|r| evaluates(&r.condition, &ctx))
            .collect()
    } else {
        vec![]
    };
    let mut found: Vec<_> = runbooks
        .iter()
        .filter_map(|rb| {
            if let Some(cond) = rb.matches.as_ref().filter(|c| evaluates(c, &ctx)) {
                return Some((rb, specificity(cond)));
            }
            matched_rules
                .iter()
                .filter(|r| rb.rule_ids.contains(&r.id))
                .map(|r| specificity(&r.condition))
                .max()
                .map(|s| (rb, s))
        })
        .collect();
    found.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.id.cmp(&b.0.id)));
    found
}

/// Podstawia wartości alertu w tytule, treści, tekstach kroków i parametrach (także zagnieżdżonych).
/// Błędny szablon zostaje jako zwykły tekst – runbook ma się pokazać mimo literówki.
pub fn render_runbook(def: &RunbookDef, ctx: &Value) -> RunbookDef {
    let mut def = def.clone();
    def.title = render(&def.title, ctx);
    def.body = def.body.as_deref().map(|b| render(b, ctx));
    for step in &mut def.steps {
        step.text = render(&step.text, ctx);
        step.params.values_mut().for_each(|v| render_value(v, ctx));
    }
    def
}

fn render(template: &str, ctx: &Value) -> String {
    condition::render_template(template, ctx).unwrap_or_else(|_| template.to_string())
}

fn render_value(v: &mut Value, ctx: &Value) {
    match v {
        Value::String(s) => *s = render(s, ctx),
        Value::Array(items) => items.iter_mut().for_each(|i| render_value(i, ctx)),
        Value::Object(map) => map.values_mut().for_each(|i| render_value(i, ctx)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_runbook;

    fn alert() -> AlertChanged {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../contracts/fixtures/topics/alerts.changed.json"
        );
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn context_exposes_raw_keys_and_canonical_wins() {
        let ctx = alert_context(&alert());
        assert_eq!(ctx["workflowId"], "WF-20261023-00042");
        assert_eq!(ctx["raw"]["error"]["code"], "TIMEOUT");
        assert_eq!(ctx["severity"], "error"); // kanoniczne, nie "ERROR" z oryginału
        assert_eq!(ctx["episode"], 1);
    }

    #[test]
    fn specificity_counts_leaves() {
        let general =
            parse_runbook("g.yaml", "matches: {type: WORKFLOW_FAILED}\nsteps: [x]").unwrap();
        let specific = parse_runbook(
            "s.yaml",
            "matches: {type: WORKFLOW_FAILED, error.code: TIMEOUT}\nsteps: [x]",
        )
        .unwrap();
        assert_eq!(specificity(general.matches.as_ref().unwrap()), 1);
        assert_eq!(specificity(specific.matches.as_ref().unwrap()), 2);
    }

    #[test]
    fn specific_runbook_first_and_params_rendered() {
        let general = parse_runbook(
            "general.yaml",
            "matches: {type: WORKFLOW_FAILED}\nsteps: [x]",
        )
        .unwrap();
        let specific = parse_runbook(
            "specific.yaml",
            "matches: {type: WORKFLOW_FAILED, error.code: TIMEOUT}\nsteps:\n  - action: retry_workflow\n    workflowId: \"{{raw.workflowId}}\"\n",
        )
        .unwrap();
        let other =
            parse_runbook("other.yaml", "matches: {type: SOMETHING_ELSE}\nsteps: [x]").unwrap();
        let runbooks = [general, other, specific];
        let found = matching(&runbooks, &[], &alert());
        assert_eq!(
            found.iter().map(|(r, _)| r.id.as_str()).collect::<Vec<_>>(),
            ["specific", "general"]
        );
        let rendered = render_runbook(found[0].0, &alert_context(&alert()));
        assert_eq!(rendered.steps[0].params["workflowId"], "WF-20261023-00042");
    }

    #[test]
    fn appendix_b_template_without_raw_prefix_is_rendered() {
        let def = parse_runbook(
            "r.yaml",
            "steps:\n  - action: retry_workflow\n    workflowId: \"{{workflowId}}\"\n",
        )
        .unwrap();
        let rendered = render_runbook(&def, &alert_context(&alert()));
        assert_eq!(rendered.steps[0].params["workflowId"], "WF-20261023-00042");
    }

    #[test]
    fn runbook_pinned_to_matching_rule() {
        let rule: RuleDef = serde_json::from_value(json!({
            "id": "settlements-prod", "name": "Settlements", "condition": { "field": "raw.workflowName", "op": "eq", "value": "order-settlement" },
            "recipients": [], "presentation": {}, "enabled": true, "version": 1
        }))
        .unwrap();
        let other_rule = RuleDef {
            id: "pricing".into(),
            condition: serde_json::from_value(
                json!({ "field": "raw.service", "op": "eq", "value": "pricing" }),
            )
            .unwrap(),
            ..rule.clone()
        };
        let pinned = parse_runbook(
            "pinned.md",
            "---\nrule_ids: [settlements-prod]\n---\nCall settlements",
        )
        .unwrap();
        let pinned_elsewhere = parse_runbook(
            "elsewhere.md",
            "---\nrule_ids: [pricing]\n---\nCall pricing",
        )
        .unwrap();
        let runbooks = [pinned, pinned_elsewhere];
        let found = matching(&runbooks, &[rule, other_rule], &alert());
        assert_eq!(
            found
                .iter()
                .map(|(r, s)| (r.id.as_str(), *s))
                .collect::<Vec<_>>(),
            [("pinned", 1)]
        );
    }
}
