//! Parsowanie plików z `config/p3` (zadania 11.1, 11.2).
//!
//! - `*.yaml|*.yml` → runbook AUTOMATIC. Format jak w Appendix B: klucze kroku
//!   `check`, `action`, `if`, `approval`, `text`, `manual`, `note`, `id`, `type`, `params`;
//!   każdy inny klucz kroku to parametr akcji (np. `url`, `workflowId`, `team`).
//! - `*.md|*.txt` → runbook MANUAL; opcjonalny nagłówek YAML między `---` (`id`, `title`,
//!   `matches`, `rule_ids`), reszta pliku to treść.
//! - `matches`: pełny warunek (`all`/`any`/`not`/`field`) albo skrót `{pole: wartość}`
//!   = AND równości; pola spoza kanonicznych dostają prefiks `raw.`.
//!
//! Tu nie sprawdzamy rejestru akcji: `automation` ustawia `Registry::classify`.

use std::collections::HashSet;
use std::path::Path;

use contracts::{ActionDef, Approval, Automation, Condition, Op, RunbookDef, RunbookKind, StepDef, StepType};
use serde_json::{Map, Value};

/// Pola kanoniczne zdarzenia (zob. `condition::event_context`); reszta to oryginał pod `raw.`.
const CANONICAL_FIELDS: [&str; 5] = ["schema", "source", "severity", "title", "fingerprint"];

/// Wczytuje runbook z pliku. Błąd zawiera nazwę pliku i (dla YAML) linię/kolumnę.
pub fn parse_runbook(file: &str, text: &str) -> Result<RunbookDef, String> {
    let path = Path::new(file);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(file);
    let result = match path.extension().and_then(|e| e.to_str()) {
        Some("yaml" | "yml") => parse_yaml(stem, text),
        Some("md" | "txt") => parse_text(stem, text),
        _ => Err("nieobsługiwane rozszerzenie (dozwolone: .yaml, .yml, .md, .txt)".into()),
    };
    result
        .map(|mut def| {
            def.source_file = Some(file.to_string());
            def
        })
        .map_err(|e| format!("{file}: {e}"))
}

/// Wczytuje akcję deklaratywną (`ActionDef` w YAML). Walidacja: `registry::validate_action`.
pub fn parse_action(file: &str, text: &str) -> Result<ActionDef, String> {
    serde_yaml::from_str(text).map_err(|e| format!("{file}: {e}"))
}

fn parse_yaml(stem: &str, text: &str) -> Result<RunbookDef, String> {
    let doc: Value = serde_yaml::from_str(text).map_err(|e| e.to_string())?;
    let obj = doc.as_object().ok_or("oczekiwano mapy na najwyższym poziomie")?;
    let mut def = header(stem, obj)?;
    let steps = obj.get("steps").and_then(Value::as_array).ok_or("brak listy `steps`")?;
    let mut ids = HashSet::new();
    for (i, raw) in steps.iter().enumerate() {
        let step = parse_step(i, raw).map_err(|e| format!("krok {}: {e}", i + 1))?;
        if !ids.insert(step.id.clone()) {
            return Err(format!("krok {}: powtórzone id `{}`", i + 1, step.id));
        }
        def.steps.push(step);
    }
    Ok(def)
}

fn parse_text(stem: &str, text: &str) -> Result<RunbookDef, String> {
    let (front, body) = split_front_matter(text);
    let front = match front {
        Some(yaml) => match serde_yaml::from_str::<Value>(yaml).map_err(|e| format!("nagłówek: {e}"))? {
            Value::Object(obj) => obj,
            _ => return Err("nagłówek: oczekiwano mapy".into()),
        },
        None => Map::new(),
    };
    let mut def = header(stem, &front)?;
    def.kind = RunbookKind::Manual;
    if !front.contains_key("title") {
        if let Some(h1) = body.lines().find_map(|l| l.strip_prefix("# ")) {
            def.title = h1.trim().to_string();
        }
    }
    def.body = Some(body.to_string());
    Ok(def)
}

/// `---\n<yaml>\n---\n<treść>` → (Some(yaml), treść); bez nagłówka → (None, cały tekst).
fn split_front_matter(text: &str) -> (Option<&str>, &str) {
    let Some(rest) = text.strip_prefix("---\n") else { return (None, text) };
    match rest.find("\n---\n") {
        Some(end) => (Some(&rest[..end]), &rest[end + 5..]),
        None => (None, text),
    }
}

/// Wspólne pola: `runbook`/`id`, `title`, `matches`, `rule_ids`.
fn header(stem: &str, obj: &Map<String, Value>) -> Result<RunbookDef, String> {
    let id = obj.get("runbook").or_else(|| obj.get("id")).map(as_text).unwrap_or_else(|| stem.to_string());
    let matches = obj.get("matches").map(parse_matches).transpose().map_err(|e| format!("matches: {e}"))?;
    let rule_ids = match obj.get("rule_ids") {
        None => vec![],
        Some(v) => serde_json::from_value(v.clone()).map_err(|e| format!("rule_ids: {e}"))?,
    };
    Ok(RunbookDef {
        title: obj.get("title").map(as_text).unwrap_or_else(|| id.clone()),
        id,
        kind: RunbookKind::Automatic,
        matches,
        rule_ids,
        body: None,
        steps: vec![],
        source_file: None,
        version: 0,
    })
}

fn parse_matches(v: &Value) -> Result<Condition, String> {
    let obj = v.as_object().ok_or("oczekiwano mapy")?;
    let condition = if ["all", "any", "not", "field"].iter().any(|k| obj.contains_key(*k)) {
        serde_json::from_value(v.clone()).map_err(|e| e.to_string())?
    } else {
        let leaves = obj
            .iter()
            .map(|(k, value)| Condition::Leaf { field: field_path(k), op: Op::Eq, value: value.clone() })
            .collect();
        Condition::All { all: leaves }
    };
    condition::compile(&condition).map_err(|e| e.to_string())?;
    Ok(condition)
}

fn field_path(key: &str) -> String {
    if key.starts_with("raw.") || CANONICAL_FIELDS.contains(&key) {
        key.to_string()
    } else {
        format!("raw.{key}")
    }
}

fn parse_step(index: usize, raw: &Value) -> Result<StepDef, String> {
    let mut step = StepDef {
        id: format!("s{}", index + 1),
        step_type: StepType::Manual,
        text: String::new(),
        action: None,
        params: Map::new(),
        approval: Approval::None,
        condition: None,
        automation: Automation::Manual,
    };
    // Krok jako sam tekst (checklista) = krok manualny.
    let obj = match raw {
        Value::String(s) => {
            step.text = s.clone();
            return Ok(step);
        }
        Value::Object(obj) => obj,
        _ => return Err("oczekiwano mapy albo tekstu".into()),
    };

    let mut explicit_type = None;
    let mut type_hint = None;
    for (key, val) in obj {
        match key.as_str() {
            "id" => step.id = as_text(val),
            "type" => explicit_type = Some(serde_json::from_value(val.clone()).map_err(|e| format!("type: {e}"))?),
            "text" | "description" => step.text = as_text(val),
            "check" => (type_hint, step.text) = (Some(StepType::Check), as_text(val)),
            "manual" => (type_hint, step.text) = (Some(StepType::Manual), as_text(val)),
            "note" => (type_hint, step.text) = (Some(StepType::Note), as_text(val)),
            "action" => step.action = Some(as_text(val)),
            "if" | "condition" => step.condition = Some(as_text(val)),
            "approval" => step.approval = parse_approval(val),
            "params" => step.params.extend(val.as_object().ok_or("params: oczekiwano mapy")?.clone()),
            "automation" => {} // wyliczane z rejestru
            _ => {
                step.params.insert(key.clone(), val.clone());
            }
        }
    }
    // `check`/`manual`/`note` mogą stać obok `text` – wtedy `text` wygrywa.
    if let Some(t) = obj.get("text").or_else(|| obj.get("description")) {
        step.text = as_text(t);
    }

    step.step_type = explicit_type.or(type_hint).unwrap_or(match (&step.action, &step.condition) {
        (Some(_), _) => StepType::Action,
        (None, Some(_)) => StepType::Note, // samo `if` = instrukcja dla człowieka
        (None, None) => StepType::Manual,
    });
    if step.text.is_empty() {
        step.text = match (&step.action, &step.condition) {
            (Some(a), _) => a.clone(),
            (None, Some(c)) => format!("Jeśli: {c}"),
            (None, None) => return Err("krok bez treści i bez akcji".into()),
        };
    }
    Ok(step)
}

/// Wszystko poza jawnym „brak” wymaga zatwierdzenia (bezpieczniejszy kierunek).
fn parse_approval(v: &Value) -> Approval {
    match v {
        Value::Null | Value::Bool(false) => Approval::None,
        Value::String(s) if matches!(s.as_str(), "none" | "no" | "false") => Approval::None,
        _ => Approval::Required,
    }
}

fn as_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const APPENDIX_B_YAML: &str = r#"
runbook: settlement-timeout
matches: { type: WORKFLOW_FAILED, "error.code": TIMEOUT }
steps:
  - check: "Is counterparty-service healthy?"
    action: http_get
    url: "http://counterparty-service/health"
  - action: retry_workflow
    workflowId: "{{workflowId}}"
    approval: none
  - if: "retry fails twice"
    action: escalate
    team: counterparty-support
    approval: required
"#;

    #[test]
    fn appendix_b_yaml_verbatim() {
        let def = parse_runbook("x/settlement-timeout.yaml", APPENDIX_B_YAML).unwrap();
        assert_eq!(def.id, "settlement-timeout");
        assert_eq!(def.kind, RunbookKind::Automatic);
        let Some(Condition::All { all }) = &def.matches else { panic!("{:?}", def.matches) };
        assert!(all.contains(&Condition::Leaf { field: "raw.type".into(), op: Op::Eq, value: "WORKFLOW_FAILED".into() }));
        assert!(all.contains(&Condition::Leaf { field: "raw.error.code".into(), op: Op::Eq, value: "TIMEOUT".into() }));

        let [s1, s2, s3] = &def.steps[..] else { panic!() };
        assert_eq!((s1.step_type, s1.text.as_str(), s1.action.as_deref()), (StepType::Check, "Is counterparty-service healthy?", Some("http_get")));
        assert_eq!(s1.params["url"], "http://counterparty-service/health");
        assert_eq!((s2.id.as_str(), s2.approval), ("s2", Approval::None));
        assert_eq!(s2.params["workflowId"], "{{workflowId}}");
        assert_eq!((s3.step_type, s3.approval), (StepType::Action, Approval::Required));
        assert_eq!(s3.condition.as_deref(), Some("retry fails twice"));
        assert_eq!(s3.params["team"], "counterparty-support");
    }

    #[test]
    fn if_without_action_is_note() {
        let def = parse_runbook("r.yaml", "steps:\n  - if: latency still high\n  - Call the on-call person\n").unwrap();
        assert_eq!(def.steps[0].step_type, StepType::Note);
        assert_eq!(def.steps[0].text, "Jeśli: latency still high");
        assert_eq!((def.steps[1].step_type, def.steps[1].text.as_str()), (StepType::Manual, "Call the on-call person"));
    }

    #[test]
    fn shell_command_is_just_params() {
        let def = parse_runbook("r.yaml", "steps:\n  - action: shell\n    command: \"rm -rf /\"\n").unwrap();
        assert_eq!(def.steps[0].action.as_deref(), Some("shell"));
        assert_eq!(def.steps[0].automation, Automation::Manual);
    }

    #[test]
    fn invalid_yaml_reports_file_and_line() {
        let err = parse_runbook("config/p3/runbooks/bad.yaml", "steps:\n  - action: [unclosed\n").unwrap_err();
        assert!(err.starts_with("config/p3/runbooks/bad.yaml: "), "{err}");
        assert!(err.contains("line"), "{err}");
    }

    #[test]
    fn duplicate_step_ids_rejected() {
        let err = parse_runbook("r.yaml", "steps:\n  - {id: a, manual: x}\n  - {id: a, manual: y}\n").unwrap_err();
        assert!(err.contains("powtórzone id"), "{err}");
    }

    #[test]
    fn text_without_header_is_manual() {
        let def = parse_runbook("p/pricing-latency.txt", "When pricing latency goes over a second...").unwrap();
        assert_eq!((def.id.as_str(), def.kind, def.matches.is_none()), ("pricing-latency", RunbookKind::Manual, true));
    }

    #[test]
    fn unknown_extension_rejected() {
        assert!(parse_runbook("r.json", "{}").unwrap_err().contains("rozszerzenie"));
    }
}
