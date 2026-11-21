//! Dopasowanie runbooków do alertu i podstawianie wartości (zadanie 11.3).

use contracts::{AlertChanged, Condition, RunbookDef};
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

/// Pasujące runbooki od najbardziej szczegółowego (remis: po id, żeby kolejność była stała).
pub fn matching<'a>(runbooks: &'a [RunbookDef], ctx: &Value) -> Vec<(&'a RunbookDef, i32)> {
    let mut found: Vec<_> = runbooks
        .iter()
        .filter_map(|rb| {
            // shortcut: `rule_ids` (przypięcie do reguły) nie jest jeszcze sprawdzane – czeka na
            // tabelę reguł P2 (otwarta kwestia w CLAUDE.md); do tego czasu liczy się tylko `matches`.
            let cond = rb.matches.as_ref()?;
            let compiled = condition::compile(cond).ok()?;
            compiled.evaluate(ctx).then(|| (rb, specificity(cond)))
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
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../contracts/fixtures/topics/alerts.changed.json");
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
        let general = parse_runbook("g.yaml", "matches: {type: WORKFLOW_FAILED}\nsteps: [x]").unwrap();
        let specific = parse_runbook("s.yaml", "matches: {type: WORKFLOW_FAILED, error.code: TIMEOUT}\nsteps: [x]").unwrap();
        assert_eq!(specificity(general.matches.as_ref().unwrap()), 1);
        assert_eq!(specificity(specific.matches.as_ref().unwrap()), 2);
    }

    #[test]
    #[ignore = "czeka na prawdziwy crate condition (P2, sekcja 6)"]
    fn specific_runbook_first_and_params_rendered() {
        let general = parse_runbook("general.yaml", "matches: {type: WORKFLOW_FAILED}\nsteps: [x]").unwrap();
        let specific = parse_runbook(
            "specific.yaml",
            "matches: {type: WORKFLOW_FAILED, error.code: TIMEOUT}\nsteps:\n  - action: retry_workflow\n    workflowId: \"{{raw.workflowId}}\"\n",
        )
        .unwrap();
        let other = parse_runbook("other.yaml", "matches: {type: SOMETHING_ELSE}\nsteps: [x]").unwrap();
        let runbooks = [general, other, specific];
        let ctx = alert_context(&alert());
        let found = matching(&runbooks, &ctx);
        assert_eq!(found.iter().map(|(r, _)| r.id.as_str()).collect::<Vec<_>>(), ["specific", "general"]);
        let rendered = render_runbook(found[0].0, &ctx);
        assert_eq!(rendered.steps[0].params["workflowId"], "WF-20261023-00042");
    }
}
