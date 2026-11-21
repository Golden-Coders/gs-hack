//! Ewaluator warunków i render szablonów (design D6). Właściciel: P2.
//!
//! PUBLICZNE API JEST ZAMROŻONE (Faza 0) – używają go `lifecycle` (P1), `router` (P2)
//! i `runbooks` (P3). P2 wymienia implementację, nie sygnatury.

use contracts::{CanonicalEvent, Condition, Explanation, Op};
use globset::{GlobBuilder, GlobMatcher};
use regex::Regex;
use serde_json::{json, Value};

const MAX_TEMPLATE_LEN: usize = 16 * 1024;
const MAX_RENDERED_LEN: usize = 64 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid condition: {0}")]
    InvalidCondition(String),
    #[error("invalid template: {0}")]
    InvalidTemplate(String),
}

/// Kontekst ewaluacji: pola kanoniczne na najwyższym poziomie + oryginał pod `raw`.
///
/// Ścieżka `severity` czyta pole kanoniczne, `raw.payload.notional` – oryginał,
/// a nazwy z `fields` (np. `service`) są dostępne bezpośrednio.
pub fn event_context(event: &CanonicalEvent) -> Value {
    let mut ctx = serde_json::Map::new();
    for (k, v) in &event.fields {
        ctx.insert(k.clone(), v.clone());
    }
    ctx.insert("event_id".into(), json!(event.event_id));
    ctx.insert("schema".into(), json!(event.schema));
    ctx.insert("source".into(), json!(event.source));
    ctx.insert("severity".into(), json!(event.severity));
    ctx.insert("title".into(), json!(event.title));
    ctx.insert("fingerprint".into(), json!(event.fingerprint));
    ctx.insert("resolved".into(), json!(event.resolved));
    ctx.insert("received_at".into(), json!(event.received_at));
    ctx.insert("raw".into(), event.raw.clone());
    Value::Object(ctx)
}

#[derive(Debug, Clone)]
enum CompiledNode {
    All(Vec<CompiledNode>),
    Any(Vec<CompiledNode>),
    Not(Box<CompiledNode>),
    Leaf(CompiledLeaf),
}

#[derive(Debug, Clone)]
struct CompiledLeaf {
    field: String,
    segments: Vec<String>,
    matcher: CompiledMatcher,
    node_label: String,
}

#[derive(Debug, Clone)]
enum CompiledMatcher {
    Eq(Value),
    Ne(Value),
    In(Vec<Value>),
    Gt(Value),
    Gte(Value),
    Lt(Value),
    Lte(Value),
    Contains(Value),
    Regex(Regex),
    Glob(GlobMatcher),
    Exists(bool),
}

/// Warunek po kompilacji (regexy, globy prekompilowane). Tanie do współdzielenia.
#[derive(Debug, Clone)]
pub struct Compiled {
    source: Condition,
    root: CompiledNode,
}

/// Kompiluje warunek. Błąd = niepoprawny warunek (np. zły regex, `in` bez tablicy).
pub fn compile(condition: &Condition) -> Result<Compiled, Error> {
    let root = compile_node(condition)?;
    Ok(Compiled {
        source: condition.clone(),
        root,
    })
}

fn compile_node(condition: &Condition) -> Result<CompiledNode, Error> {
    match condition {
        Condition::All { all } => {
            let children = all
                .iter()
                .map(compile_node)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(CompiledNode::All(children))
        }
        Condition::Any { any } => {
            let children = any
                .iter()
                .map(compile_node)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(CompiledNode::Any(children))
        }
        Condition::Not { not } => {
            let child = compile_node(not)?;
            Ok(CompiledNode::Not(Box::new(child)))
        }
        Condition::Leaf { field, op, value } => {
            let field_trimmed = field.trim();
            if field_trimmed.is_empty() {
                return Err(Error::InvalidCondition("field name cannot be empty".into()));
            }
            let segments = field_trimmed
                .split('.')
                .map(std::string::ToString::to_string)
                .collect::<Vec<_>>();
            let matcher = compile_matcher(*op, value)?;
            let node_label = format_leaf_node(field_trimmed, *op, value);
            Ok(CompiledNode::Leaf(CompiledLeaf {
                field: field_trimmed.to_string(),
                segments,
                matcher,
                node_label,
            }))
        }
    }
}

fn compile_matcher(op: Op, value: &Value) -> Result<CompiledMatcher, Error> {
    match op {
        Op::Eq => {
            if value.is_null() {
                return Err(Error::InvalidCondition(
                    "`eq` value cannot be null (use `exists` operator instead)".into(),
                ));
            }
            Ok(CompiledMatcher::Eq(value.clone()))
        }
        Op::Ne => {
            if value.is_null() {
                return Err(Error::InvalidCondition(
                    "`ne` value cannot be null (use `exists` operator instead)".into(),
                ));
            }
            Ok(CompiledMatcher::Ne(value.clone()))
        }
        Op::In => match value {
            Value::Array(arr) => Ok(CompiledMatcher::In(arr.clone())),
            _ => Err(Error::InvalidCondition(
                "`in` operator requires an array value".into(),
            )),
        },
        Op::Gt => {
            validate_orderable(op, value)?;
            Ok(CompiledMatcher::Gt(value.clone()))
        }
        Op::Gte => {
            validate_orderable(op, value)?;
            Ok(CompiledMatcher::Gte(value.clone()))
        }
        Op::Lt => {
            validate_orderable(op, value)?;
            Ok(CompiledMatcher::Lt(value.clone()))
        }
        Op::Lte => {
            validate_orderable(op, value)?;
            Ok(CompiledMatcher::Lte(value.clone()))
        }
        Op::Contains => {
            if value.is_null() || value.is_object() || value.is_array() {
                return Err(Error::InvalidCondition(
                    "`contains` operator requires a scalar non-null value".into(),
                ));
            }
            Ok(CompiledMatcher::Contains(value.clone()))
        }
        Op::Regex => {
            let pattern = value.as_str().ok_or_else(|| {
                Error::InvalidCondition("`regex` operator requires a string pattern".into())
            })?;
            let re = regex::RegexBuilder::new(pattern)
                .size_limit(1 << 20)
                .build()
                .map_err(|e| Error::InvalidCondition(format!("invalid regex `{pattern}`: {e}")))?;
            Ok(CompiledMatcher::Regex(re))
        }
        Op::Glob => {
            let pattern = value.as_str().ok_or_else(|| {
                Error::InvalidCondition("`glob` operator requires a string pattern".into())
            })?;
            let glob = GlobBuilder::new(pattern)
                .literal_separator(false)
                .build()
                .map_err(|e| Error::InvalidCondition(format!("invalid glob `{pattern}`: {e}")))?
                .compile_matcher();
            Ok(CompiledMatcher::Glob(glob))
        }
        Op::Exists => match value {
            Value::Null => Ok(CompiledMatcher::Exists(true)),
            Value::Bool(b) => Ok(CompiledMatcher::Exists(*b)),
            _ => Err(Error::InvalidCondition(
                "`exists` operator expects boolean or omitted/null value".into(),
            )),
        },
    }
}

fn validate_orderable(op: Op, value: &Value) -> Result<(), Error> {
    if value.is_number() || value.is_string() {
        Ok(())
    } else {
        Err(Error::InvalidCondition(format!(
            "`{}` operator requires a number or string value",
            op_str(op)
        )))
    }
}

fn op_str(op: Op) -> &'static str {
    match op {
        Op::Eq => "eq",
        Op::Ne => "ne",
        Op::In => "in",
        Op::Gt => "gt",
        Op::Gte => "gte",
        Op::Lt => "lt",
        Op::Lte => "lte",
        Op::Contains => "contains",
        Op::Regex => "regex",
        Op::Exists => "exists",
        Op::Glob => "glob",
    }
}

fn format_value_for_node(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn format_leaf_node(field: &str, op: Op, value: &Value) -> String {
    if op == Op::Exists && value.is_null() {
        format!("{field} exists")
    } else {
        format!("{field} {} {}", op_str(op), format_value_for_node(value))
    }
}

/// Resolves a dot-separated field path in `ctx`.
/// Also supports direct keys containing dots as a fallback and array indices.
fn resolve_path<'a>(ctx: &'a Value, field: &str, segments: &[String]) -> Option<&'a Value> {
    if let Value::Object(map) = ctx {
        if let Some(direct) = map.get(field) {
            return Some(direct);
        }
    }
    resolve_segments(ctx, segments)
}

fn resolve_segments<'a>(current: &'a Value, segments: &[String]) -> Option<&'a Value> {
    if segments.is_empty() {
        return Some(current);
    }
    match current {
        Value::Object(map) => {
            let first = &segments[0];
            if let Some(next) = map.get(first) {
                if let Some(found) = resolve_segments(next, &segments[1..]) {
                    return Some(found);
                }
            }
            if segments.len() > 1 {
                let joined = segments.join(".");
                if let Some(direct) = map.get(&joined) {
                    return Some(direct);
                }
            }
            None
        }
        Value::Array(arr) => {
            let idx: usize = segments[0].parse().ok()?;
            let next = arr.get(idx)?;
            resolve_segments(next, &segments[1..])
        }
        _ => None,
    }
}

fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(na), Value::Number(nb)) => {
            if let (Some(ia), Some(ib)) = (na.as_i64(), nb.as_i64()) {
                return ia == ib;
            }
            if let (Some(ua), Some(ub)) = (na.as_u64(), nb.as_u64()) {
                return ua == ub;
            }
            match (na.as_f64(), nb.as_f64()) {
                (Some(fa), Some(fb)) => fa == fb,
                _ => false,
            }
        }
        (Value::String(sa), Value::String(sb)) => {
            if sa == sb {
                return true;
            }
            if let (Some(ra), Some(rb)) = (severity_rank(sa), severity_rank(sb)) {
                return ra == rb;
            }
            false
        }
        _ => a == b,
    }
}

fn severity_rank(s: &str) -> Option<u8> {
    if s.eq_ignore_ascii_case("info") {
        Some(0)
    } else if s.eq_ignore_ascii_case("warning") || s.eq_ignore_ascii_case("warn") {
        Some(1)
    } else if s.eq_ignore_ascii_case("error") {
        Some(2)
    } else if s.eq_ignore_ascii_case("critical") || s.eq_ignore_ascii_case("crit") {
        Some(3)
    } else {
        None
    }
}

fn compare_values(actual: &Value, expected: &Value) -> Option<std::cmp::Ordering> {
    if let (Some(na), Some(nb)) = (actual.as_f64(), expected.as_f64()) {
        return na.partial_cmp(&nb);
    }
    if let (Some(sa), Some(sb)) = (actual.as_str(), expected.as_str()) {
        if let (Some(ra), Some(rb)) = (severity_rank(sa), severity_rank(sb)) {
            return Some(ra.cmp(&rb));
        }
        return Some(sa.cmp(sb));
    }
    None
}

impl CompiledLeaf {
    fn eval_with_actual<'a>(&self, ctx: &'a Value) -> (bool, Option<&'a Value>) {
        let resolved = resolve_path(ctx, &self.field, &self.segments);
        let actual_present = resolved.filter(|v| !v.is_null());

        if let CompiledMatcher::Exists(expect_exists) = self.matcher {
            let is_present = actual_present.is_some();
            return (is_present == expect_exists, actual_present);
        }

        let Some(actual) = actual_present else {
            return (false, None);
        };

        let matched = match &self.matcher {
            CompiledMatcher::Eq(expected) => values_equal(actual, expected),
            CompiledMatcher::Ne(expected) => !values_equal(actual, expected),
            CompiledMatcher::In(list) => match actual {
                Value::Array(actual_arr) => actual_arr
                    .iter()
                    .any(|a| list.iter().any(|e| values_equal(a, e))),
                _ => list.iter().any(|e| values_equal(actual, e)),
            },
            CompiledMatcher::Gt(expected) => {
                compare_values(actual, expected) == Some(std::cmp::Ordering::Greater)
            }
            CompiledMatcher::Gte(expected) => matches!(
                compare_values(actual, expected),
                Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
            ),
            CompiledMatcher::Lt(expected) => {
                compare_values(actual, expected) == Some(std::cmp::Ordering::Less)
            }
            CompiledMatcher::Lte(expected) => matches!(
                compare_values(actual, expected),
                Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
            ),
            CompiledMatcher::Contains(expected) => match actual {
                Value::String(s) => expected.as_str().is_some_and(|sub| s.contains(sub)),
                Value::Array(arr) => arr.iter().any(|elem| values_equal(elem, expected)),
                _ => false,
            },
            CompiledMatcher::Regex(re) => actual.as_str().is_some_and(|s| re.is_match(s)),
            CompiledMatcher::Glob(glob) => actual.as_str().is_some_and(|s| glob.is_match(s)),
            CompiledMatcher::Exists(_) => unreachable!(),
        };

        (matched, Some(actual))
    }
}

impl CompiledNode {
    fn evaluate(&self, ctx: &Value) -> bool {
        match self {
            CompiledNode::All(children) => children.iter().all(|c| c.evaluate(ctx)),
            CompiledNode::Any(children) => children.iter().any(|c| c.evaluate(ctx)),
            CompiledNode::Not(child) => !child.evaluate(ctx),
            CompiledNode::Leaf(leaf) => leaf.eval_with_actual(ctx).0,
        }
    }

    fn explain(&self, ctx: &Value) -> Explanation {
        match self {
            CompiledNode::All(children) => {
                let explained: Vec<Explanation> = children.iter().map(|c| c.explain(ctx)).collect();
                let result = explained.iter().all(|e| e.result);
                Explanation {
                    result,
                    node: "all".into(),
                    actual: None,
                    children: explained,
                }
            }
            CompiledNode::Any(children) => {
                let explained: Vec<Explanation> = children.iter().map(|c| c.explain(ctx)).collect();
                let result = explained.iter().any(|e| e.result);
                Explanation {
                    result,
                    node: "any".into(),
                    actual: None,
                    children: explained,
                }
            }
            CompiledNode::Not(child) => {
                let child_exp = child.explain(ctx);
                let result = !child_exp.result;
                Explanation {
                    result,
                    node: "not".into(),
                    actual: None,
                    children: vec![child_exp],
                }
            }
            CompiledNode::Leaf(leaf) => {
                let (result, actual) = leaf.eval_with_actual(ctx);
                Explanation {
                    result,
                    node: leaf.node_label.clone(),
                    actual: actual.cloned(),
                    children: vec![],
                }
            }
        }
    }
}

impl Compiled {
    /// Czy kontekst (zob. [`event_context`]) spełnia warunek.
    pub fn evaluate(&self, ctx: &Value) -> bool {
        self.root.evaluate(ctx)
    }

    /// Wynik każdego węzła drzewa – do „testuj na zdarzeniu”.
    pub fn explain(&self, ctx: &Value) -> Explanation {
        self.root.explain(ctx)
    }

    /// Warunek źródłowy.
    pub fn source(&self) -> &Condition {
        &self.source
    }
}

/// Sprawdza czy wyrażenie wewnątrz `{{ ... }}` jest prostą, bezpieczną ścieżką do pola
/// (np. `raw.payload.currency`, `fields.0.name`, `severity`).
/// Odrzuca próby wywołania funkcji, dostępu do plików, zmiennych środowiskowych lub wykonania kodu.
fn validate_interpolation_expr(expr: &str) -> Result<(), Error> {
    let trimmed = expr.trim();
    if trimmed.is_empty() {
        return Err(Error::InvalidTemplate("empty interpolation `{{}}`".into()));
    }

    if trimmed.contains("..")
        || trimmed.contains("__")
        || trimmed.contains('/')
        || trimmed.contains('\\')
    {
        return Err(Error::InvalidTemplate(format!(
            "forbidden path or dunder in template expression `{trimmed}`"
        )));
    }

    for ch in trimmed.chars() {
        if !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' || ch == '-') {
            return Err(Error::InvalidTemplate(format!(
                "forbidden character `{ch}` in template expression `{trimmed}`"
            )));
        }
    }

    if trimmed.starts_with('.') || trimmed.ends_with('.') {
        return Err(Error::InvalidTemplate(format!(
            "invalid field path `{trimmed}`"
        )));
    }

    for segment in trimmed.split('.') {
        if segment.is_empty() {
            return Err(Error::InvalidTemplate(format!(
                "empty path segment in `{trimmed}`"
            )));
        }
        let lower = segment.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "env"
                | "environ"
                | "process"
                | "system"
                | "exec"
                | "eval"
                | "include"
                | "import"
                | "extends"
                | "open"
                | "file"
                | "fs"
                | "std"
                | "self"
                | "super"
                | "loop"
                | "range"
                | "dict"
                | "namespace"
                | "cycler"
                | "joiner"
                | "debug"
        ) {
            return Err(Error::InvalidTemplate(format!(
                "forbidden identifier `{segment}` in template expression"
            )));
        }
    }

    Ok(())
}

/// Parsuje i waliduje szablon, zwracając listę tokenów lub błąd.
fn parse_template_tokens(template: &str) -> Result<Vec<TemplateToken<'_>>, Error> {
    if template.len() > MAX_TEMPLATE_LEN {
        return Err(Error::InvalidTemplate(format!(
            "template length {} exceeds maximum allowed {} bytes",
            template.len(),
            MAX_TEMPLATE_LEN
        )));
    }

    if template.contains("{%") || template.contains("%}") {
        return Err(Error::InvalidTemplate(
            "control block tags `{% ... %}` are not allowed in templates".into(),
        ));
    }
    if template.contains("{#") || template.contains("#}") {
        return Err(Error::InvalidTemplate(
            "comment tags `{# ... #}` are not allowed in templates".into(),
        ));
    }

    let mut tokens = Vec::new();
    let mut rest = template;

    while let Some(start) = rest.find("{{") {
        if start > 0 {
            let literal = &rest[..start];
            if literal.contains("}}") {
                return Err(Error::InvalidTemplate("unmatched `}}` in template".into()));
            }
            tokens.push(TemplateToken::Literal(literal));
        }
        let after_open = &rest[start + 2..];
        let Some(end) = after_open.find("}}") else {
            return Err(Error::InvalidTemplate("unclosed `{{` in template".into()));
        };
        let expr = &after_open[..end];
        if expr.contains("{{") {
            return Err(Error::InvalidTemplate("nested `{{` in template".into()));
        }
        validate_interpolation_expr(expr)?;
        tokens.push(TemplateToken::Field(expr.trim()));
        rest = &after_open[end + 2..];
    }

    if rest.contains("}}") {
        return Err(Error::InvalidTemplate("unmatched `}}` in template".into()));
    }
    if !rest.is_empty() {
        tokens.push(TemplateToken::Literal(rest));
    }

    if !template.contains('-') {
        let mut env = minijinja::Environment::empty();
        env.set_recursion_limit(16);
        env.add_template("tpl", template)
            .map_err(|e| Error::InvalidTemplate(format!("syntax error: {e}")))?;
    }

    Ok(tokens)
}

enum TemplateToken<'a> {
    Literal(&'a str),
    Field(&'a str),
}

fn value_to_template_string(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Array(_) | Value::Object(_) => v.to_string(),
    }
}

/// Sprawdza szablon przy zapisie (składnia, brak niedozwolonych konstrukcji).
pub fn validate_template(template: &str) -> Result<(), Error> {
    let _ = parse_template_tokens(template)?;
    Ok(())
}

/// Renderuje szablon `{{ścieżka}}` w sandboxie. Brakująca zmienna = pusty tekst.
pub fn render_template(template: &str, ctx: &Value) -> Result<String, Error> {
    let tokens = parse_template_tokens(template)?;
    let mut out = String::with_capacity(template.len());

    for token in tokens {
        match token {
            TemplateToken::Literal(lit) => {
                if out.len() + lit.len() > MAX_RENDERED_LEN {
                    return Err(Error::InvalidTemplate(format!(
                        "rendered output exceeds maximum size of {} bytes",
                        MAX_RENDERED_LEN
                    )));
                }
                out.push_str(lit);
            }
            TemplateToken::Field(path) => {
                let segments: Vec<String> = path
                    .split('.')
                    .map(std::string::ToString::to_string)
                    .collect();
                if let Some(val) = resolve_path(ctx, path, &segments) {
                    let rendered_val = value_to_template_string(val);
                    if out.len() + rendered_val.len() > MAX_RENDERED_LEN {
                        return Err(Error::InvalidTemplate(format!(
                            "rendered output exceeds maximum size of {} bytes",
                            MAX_RENDERED_LEN
                        )));
                    }
                    out.push_str(&rendered_val);
                }
            }
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use contracts::{CanonicalEvent, Condition, Explanation, Op, Severity};
    use serde_json::{json, Map};
    use std::time::Instant;
    use uuid::Uuid;

    fn sample_trade_event(notional: i64) -> CanonicalEvent {
        let mut fields = Map::new();
        fields.insert("desk".into(), json!("rates"));
        fields.insert("tradeId".into(), json!("T-998877"));
        CanonicalEvent {
            event_id: Uuid::nil(),
            schema: "trade.rejected.v2".into(),
            source: "kafka:trades".into(),
            severity: Severity::Critical,
            title: "trade.rejected.v2".into(),
            fingerprint: "trade.rejected.v2|T-998877".into(),
            resolved: false,
            fields,
            raw: json!({
                "meta": {
                    "schema": "trade.rejected.v2",
                    "producer": "trade-gateway",
                    "ts": 1792833245000_i64
                },
                "payload": {
                    "tradeId": "T-998877",
                    "desk": "rates",
                    "notional": notional,
                    "currency": "EUR",
                    "reason": "LIMIT_BREACH",
                    "tags": ["urgent", "tier1"]
                }
            }),
            received_at: chrono::DateTime::UNIX_EPOCH,
            mapping_version: 1,
        }
    }

    #[test]
    fn evaluate_rates_rule_and_all_operators() {
        let rates_cond: Condition = serde_yaml::from_str(
            r#"
all:
  - { field: schema, op: glob, value: "trade.rejected.*" }
  - { field: raw.payload.desk, op: eq, value: rates }
  - { field: raw.payload.notional, op: gt, value: 10000000 }
"#,
        )
        .unwrap();

        let compiled = compile(&rates_cond).unwrap();
        assert_eq!(compiled.source(), &rates_cond);

        let ev_match = sample_trade_event(25_000_000);
        let ctx_match = event_context(&ev_match);
        assert!(compiled.evaluate(&ctx_match));

        let ev_low = sample_trade_event(5_000_000);
        let ctx_low = event_context(&ev_low);
        assert!(!compiled.evaluate(&ctx_low));
    }

    #[test]
    fn evaluate_nested_and_or_not_settlements() {
        let cond: Condition = serde_yaml::from_str(
            r#"
all:
  - { field: environment, op: eq, value: prod }
  - any:
      - { field: raw.error.code, op: eq, value: TIMEOUT }
      - { field: raw.error.code, op: eq, value: REJECTED }
  - not: { field: raw.step, op: eq, value: ignored-step }
"#,
        )
        .unwrap();
        let compiled = compile(&cond).unwrap();

        let ctx = json!({
            "environment": "prod",
            "raw": {
                "step": "validate-counterparty",
                "error": { "code": "REJECTED" }
            }
        });
        assert!(compiled.evaluate(&ctx));

        let ctx_dev = json!({
            "environment": "dev",
            "raw": {
                "step": "validate-counterparty",
                "error": { "code": "REJECTED" }
            }
        });
        assert!(!compiled.evaluate(&ctx_dev));

        let ctx_ignored = json!({
            "environment": "prod",
            "raw": {
                "step": "ignored-step",
                "error": { "code": "TIMEOUT" }
            }
        });
        assert!(!compiled.evaluate(&ctx_ignored));
    }

    #[test]
    fn evaluate_every_operator() {
        let ctx = json!({
            "schema": "trade.rejected.v2",
            "severity": "error",
            "service": "pricing-engine-7",
            "count": 42,
            "ratio": 42.0,
            "null_field": null,
            "tags": ["fx", "rates", "urgent"],
            "message": "Counterparty service did not respond in 5000 ms",
            "raw": {
                "dotted.key": "direct_hit",
                "items": [{"name": "first"}, {"name": "second"}]
            }
        });

        let check = |field: &str, op: Op, value: Value| -> bool {
            let cond = Condition::Leaf {
                field: field.into(),
                op,
                value,
            };
            compile(&cond).unwrap().evaluate(&ctx)
        };

        assert!(check("count", Op::Eq, json!(42)));
        assert!(check("count", Op::Eq, json!(42.0)));
        assert!(check("ratio", Op::Eq, json!(42)));
        assert!(!check("count", Op::Eq, json!(43)));
        assert!(check("count", Op::Ne, json!(43)));
        assert!(!check("count", Op::Ne, json!(42)));

        assert!(check(
            "schema",
            Op::In,
            json!(["workflow.v1", "trade.rejected.v2"])
        ));
        assert!(!check(
            "schema",
            Op::In,
            json!(["workflow.v1", "alertmanager"])
        ));
        assert!(check("tags", Op::In, json!(["rates", "equities"])));

        assert!(check("count", Op::Gt, json!(41)));
        assert!(!check("count", Op::Gt, json!(42)));
        assert!(check("count", Op::Gte, json!(42)));
        assert!(check("count", Op::Lt, json!(43)));
        assert!(!check("count", Op::Lt, json!(42)));
        assert!(check("count", Op::Lte, json!(42)));

        assert!(check("severity", Op::Gte, json!("warning")));
        assert!(check("severity", Op::Gte, json!("WARN")));
        assert!(check("severity", Op::Gt, json!("warning")));
        assert!(check("severity", Op::Gt, json!("info")));
        assert!(check("severity", Op::Lte, json!("error")));
        assert!(check("severity", Op::Lt, json!("critical")));
        assert!(!check("severity", Op::Gte, json!("critical")));

        assert!(check("message", Op::Contains, json!("5000 ms")));
        assert!(!check("message", Op::Contains, json!("9999 ms")));
        assert!(check("tags", Op::Contains, json!("urgent")));
        assert!(!check("tags", Op::Contains, json!("missing")));

        assert!(check("message", Op::Regex, json!(r"respond in \d+ ms$")));
        assert!(!check("message", Op::Regex, json!(r"^respond")));

        assert!(check("schema", Op::Glob, json!("trade.rejected.*")));
        assert!(check("service", Op::Glob, json!("pricing-engine-*")));
        assert!(!check("service", Op::Glob, json!("payments-*")));

        assert!(check("service", Op::Exists, Value::Null));
        assert!(check("service", Op::Exists, json!(true)));
        assert!(!check("service", Op::Exists, json!(false)));

        assert!(check("raw.dotted.key", Op::Eq, json!("direct_hit")));
        assert!(check("raw.items.1.name", Op::Eq, json!("second")));
    }

    #[test]
    fn missing_and_null_fields_behavior() {
        let ctx = json!({
            "present": "ok",
            "null_field": null
        });

        for op in [
            Op::Eq,
            Op::Ne,
            Op::Gt,
            Op::Gte,
            Op::Lt,
            Op::Lte,
            Op::Contains,
            Op::Regex,
            Op::Glob,
        ] {
            let cond_missing = Condition::Leaf {
                field: "nonexistent.deep.path".into(),
                op,
                value: json!("anything"),
            };
            assert!(
                !compile(&cond_missing).unwrap().evaluate(&ctx),
                "missing field should evaluate to false for {op:?}"
            );

            let cond_null = Condition::Leaf {
                field: "null_field".into(),
                op,
                value: json!("anything"),
            };
            assert!(
                !compile(&cond_null).unwrap().evaluate(&ctx),
                "null field should evaluate to false for {op:?}"
            );
        }

        let cond_in = Condition::Leaf {
            field: "nonexistent".into(),
            op: Op::In,
            value: json!(["a", "b"]),
        };
        assert!(!compile(&cond_in).unwrap().evaluate(&ctx));

        let exists_true = Condition::Leaf {
            field: "nonexistent".into(),
            op: Op::Exists,
            value: Value::Null,
        };
        assert!(!compile(&exists_true).unwrap().evaluate(&ctx));

        let exists_false = Condition::Leaf {
            field: "nonexistent".into(),
            op: Op::Exists,
            value: json!(false),
        };
        assert!(compile(&exists_false).unwrap().evaluate(&ctx));

        let null_exists = Condition::Leaf {
            field: "null_field".into(),
            op: Op::Exists,
            value: json!(true),
        };
        assert!(!compile(&null_exists).unwrap().evaluate(&ctx));
    }

    #[test]
    fn compile_rejects_invalid_conditions() {
        let bad_regex = Condition::Leaf {
            field: "title".into(),
            op: Op::Regex,
            value: json!("[unclosed"),
        };
        assert!(matches!(
            compile(&bad_regex),
            Err(Error::InvalidCondition(_))
        ));

        let bad_glob = Condition::Leaf {
            field: "schema".into(),
            op: Op::Glob,
            value: json!("[unclosed"),
        };
        assert!(matches!(
            compile(&bad_glob),
            Err(Error::InvalidCondition(_))
        ));

        let bad_in = Condition::Leaf {
            field: "schema".into(),
            op: Op::In,
            value: json!("not-an-array"),
        };
        assert!(matches!(compile(&bad_in), Err(Error::InvalidCondition(_))));

        let bad_gt = Condition::Leaf {
            field: "count".into(),
            op: Op::Gt,
            value: json!(true),
        };
        assert!(matches!(compile(&bad_gt), Err(Error::InvalidCondition(_))));

        let empty_field = Condition::Leaf {
            field: "   ".into(),
            op: Op::Eq,
            value: json!("x"),
        };
        assert!(matches!(
            compile(&empty_field),
            Err(Error::InvalidCondition(_))
        ));
    }

    #[test]
    fn explain_rates_rule_with_unmet_threshold() {
        let rates_cond: Condition = serde_yaml::from_str(
            r#"
all:
  - { field: schema, op: glob, value: "trade.rejected.*" }
  - { field: raw.payload.desk, op: eq, value: rates }
  - { field: raw.payload.notional, op: gt, value: 10000000 }
"#,
        )
        .unwrap();

        let compiled = compile(&rates_cond).unwrap();
        let ev = sample_trade_event(5_000_000);
        let ctx = event_context(&ev);

        let exp = compiled.explain(&ctx);
        assert_eq!(
            exp,
            Explanation {
                result: false,
                node: "all".into(),
                actual: None,
                children: vec![
                    Explanation {
                        result: true,
                        node: "schema glob trade.rejected.*".into(),
                        actual: Some(json!("trade.rejected.v2")),
                        children: vec![],
                    },
                    Explanation {
                        result: true,
                        node: "raw.payload.desk eq rates".into(),
                        actual: Some(json!("rates")),
                        children: vec![],
                    },
                    Explanation {
                        result: false,
                        node: "raw.payload.notional gt 10000000".into(),
                        actual: Some(json!(5_000_000)),
                        children: vec![],
                    },
                ],
            }
        );
    }

    #[test]
    fn scenario_title_template_rendering() {
        let ev = sample_trade_event(25_000_000);
        let ctx = event_context(&ev);
        let tpl =
            "Rejected {{raw.payload.currency}} {{raw.payload.notional}} — {{raw.payload.reason}}";
        validate_template(tpl).unwrap();
        let rendered = render_template(tpl, &ctx).unwrap();
        assert_eq!(rendered, "Rejected EUR 25000000 — LIMIT_BREACH");
    }

    #[test]
    fn scenario_missing_variable_in_template() {
        let ev = sample_trade_event(25_000_000);
        let ctx = event_context(&ev);
        let tpl = "Alert: {{raw.payload.currency}} [{{raw.payload.nonexistent}}] ({{missing.deep.field}})";
        validate_template(tpl).unwrap();
        let rendered = render_template(tpl, &ctx).unwrap();
        assert_eq!(rendered, "Alert: EUR [] ()");
    }

    #[test]
    fn scenario_template_does_not_execute_code() {
        let dangerous_templates = [
            "{% include '/etc/passwd' %}",
            "{% import 'os' as os %}",
            "{% for x in range(1000000) %}x{% endfor %}",
            "{{ env.SECRET_KEY }}",
            "{{ process.env.SECRET }}",
            "{{ system('cat /etc/passwd') }}",
            "{{ exec('rm -rf /') }}",
            "{{ open('/etc/passwd') }}",
            "{{ __class__.__mro__ }}",
            "{{ ../../etc/passwd }}",
            "{{ foo | debug }}",
            "{{ range(9999999) }}",
            "{{ unclosed",
            "unopened }}",
        ];

        let ctx = json!({"env": {"SECRET_KEY": "leaked"}});
        for tpl in dangerous_templates {
            assert!(
                matches!(validate_template(tpl), Err(Error::InvalidTemplate(_))),
                "dangerous template should be rejected by validate_template: `{tpl}`"
            );
            assert!(
                matches!(render_template(tpl, &ctx), Err(Error::InvalidTemplate(_))),
                "dangerous template should be rejected by render_template: `{tpl}`"
            );
        }
    }

    #[test]
    fn template_size_limits() {
        let huge_tpl = "a".repeat(MAX_TEMPLATE_LEN + 1);
        assert!(matches!(
            validate_template(&huge_tpl),
            Err(Error::InvalidTemplate(_))
        ));

        let big_val = "x".repeat(10_000);
        let ctx = json!({ "big": big_val });
        let tpl = "{{big}}{{big}}{{big}}{{big}}{{big}}{{big}}{{big}}";
        validate_template(tpl).unwrap();
        assert!(matches!(
            render_template(tpl, &ctx),
            Err(Error::InvalidTemplate(_))
        ));
    }

    #[test]
    fn benchmark_100_rules_evaluation() {
        let mut rules = Vec::with_capacity(100);
        for i in 0..100 {
            let cond: Condition = match i % 4 {
                0 => serde_yaml::from_str(&format!(
                    r#"
all:
  - {{ field: schema, op: glob, value: "trade.rejected.*" }}
  - {{ field: raw.payload.desk, op: eq, value: rates }}
  - {{ field: raw.payload.notional, op: gt, value: {} }}
"#,
                    10_000_000 + i * 100_000
                ))
                .unwrap(),
                1 => serde_yaml::from_str(
                    r#"
all:
  - { field: schema, op: eq, value: workflow.v1 }
  - { field: raw.environment, op: eq, value: prod }
  - any:
      - { field: raw.error.code, op: eq, value: TIMEOUT }
      - { field: raw.error.code, op: eq, value: REJECTED }
"#,
                )
                .unwrap(),
                2 => serde_yaml::from_str(
                    r#"
all:
  - { field: schema, op: eq, value: alertmanager }
  - { field: service, op: eq, value: payments-api }
  - { field: severity, op: in, value: [error, critical] }
"#,
                )
                .unwrap(),
                _ => serde_yaml::from_str(
                    r#"
all:
  - { field: service, op: glob, value: "pricing-engine-*" }
  - { field: severity, op: gte, value: warning }
  - { field: title, op: regex, value: "trade|pricing|timeout" }
"#,
                )
                .unwrap(),
            };
            rules.push(compile(&cond).unwrap());
        }

        let ev = sample_trade_event(20_000_000);
        let ctx = event_context(&ev);

        let iterations = 1_000;
        let start = Instant::now();
        let mut matched_count = 0usize;
        for _ in 0..iterations {
            for rule in &rules {
                if rule.evaluate(&ctx) {
                    matched_count += 1;
                }
            }
        }
        let elapsed = start.elapsed();
        let total_rule_evals = iterations * rules.len();
        let per_event_100_rules_us = elapsed.as_secs_f64() * 1_000_000.0 / (iterations as f64);
        let per_rule_ns = elapsed.as_nanos() as f64 / (total_rule_evals as f64);

        println!(
            "BENCHMARK 100 rules x {} events: total={:?}, per_event(100 rules)={:.2} us, per_rule={:.1} ns, matched={}",
            iterations, elapsed, per_event_100_rules_us, per_rule_ns, matched_count
        );

        assert!(matched_count > 0);
        assert!(per_event_100_rules_us < 5_000.0);
    }

    #[test]
    fn seed_rules_and_mappings_integration() {
        let trade_cond: Condition = serde_yaml::from_str(
            r#"
field: raw.payload.notional
op: gt
value: 10000000
"#,
        )
        .unwrap();
        let ev_trade = sample_trade_event(25_000_000);
        assert!(compile(&trade_cond)
            .unwrap()
            .evaluate(&event_context(&ev_trade)));

        let am_resolved_cond: Condition = serde_yaml::from_str(
            r#"
field: raw.status
op: eq
value: resolved
"#,
        )
        .unwrap();
        let compiled_am = compile(&am_resolved_cond).unwrap();
        assert!(compiled_am.evaluate(&json!({"raw": {"status": "resolved"}})));
        assert!(!compiled_am.evaluate(&json!({"raw": {"status": "firing"}})));

        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
        for rule_file in [
            "rates-big-rejections.yaml",
            "settlements-prod.yaml",
            "anna-payments-critical.yaml",
            "pricing-warnings.yaml",
        ] {
            let path = format!("{root}/config/rules/{rule_file}");
            if let Ok(content) = std::fs::read_to_string(&path) {
                let rule: contracts::RuleDef = serde_yaml::from_str(&content).unwrap();
                let _compiled = compile(&rule.condition).unwrap();
                if let Some(title) = &rule.presentation.title {
                    validate_template(title).unwrap();
                }
                if let Some(body) = &rule.presentation.body {
                    validate_template(body).unwrap();
                }
            }
        }
    }

    #[test]
    fn oversized_regex_is_rejected() {
        let cond = Condition::Leaf {
            field: "message".into(),
            op: Op::Regex,
            value: json!("a{1000}{1000}"),
        };
        let err = compile(&cond).unwrap_err();
        assert!(matches!(err, Error::InvalidCondition(_)), "{err}");
    }
}
