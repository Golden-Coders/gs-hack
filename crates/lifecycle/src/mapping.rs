//! Mapping schematu -> zdarzenie kanoniczne (czysta logika, bez IO).

use std::collections::BTreeMap;

use anyhow::{bail, Context, Result};
use contracts::{formats, CanonicalEvent, Envelope, MappingDef, Severity, SeveritySpec};
use serde_json::{Map, Value};
use serde_json_path::JsonPath;

enum SeverityCompiled {
    Const(Severity),
    Path {
        path: JsonPath,
        map: BTreeMap<String, Severity>,
        default: Severity,
    },
    Rules {
        rules: Vec<(condition::Compiled, Severity)>,
        default: Severity,
    },
}

enum TitleSpec {
    Template(String),
    Path(JsonPath),
}

/// Mapping z prekompilowanymi ścieżkami i warunkami.
pub struct CompiledMapping {
    pub def: MappingDef,
    pub version: i64,
    fields: Vec<(String, JsonPath)>,
    title: Option<TitleSpec>,
    severity: SeverityCompiled,
    fingerprint: Vec<JsonPath>,
    resolved: Option<condition::Compiled>,
}

fn parse_path(p: &str) -> Result<JsonPath> {
    JsonPath::parse(p).with_context(|| format!("invalid JSONPath `{p}`"))
}

/// Sprawdza, czy tekst YAML zawiera kotwice (`&name`) lub aliasy (`*name`) poza cytatami i komentarzami.
fn has_yaml_anchor_or_alias(s: &str) -> bool {
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut in_single = false;
    let mut in_double = false;
    while i < bytes.len() {
        let b = bytes[i];
        if in_single {
            if b == b'\'' {
                if bytes.get(i + 1) == Some(&b'\'') {
                    i += 2;
                    continue;
                }
                in_single = false;
            }
            i += 1;
            continue;
        }
        if in_double {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == b'"' {
                in_double = false;
            }
            i += 1;
            continue;
        }
        let prev_is_boundary = i == 0
            || matches!(
                bytes[i - 1],
                b' ' | b'\t' | b'\n' | b'\r' | b':' | b',' | b'[' | b'{' | b'-' | b'?'
            );
        match b {
            b'\'' => in_single = true,
            b'"' => in_double = true,
            b'#' if i == 0 || matches!(bytes[i - 1], b' ' | b'\t' | b'\n' | b'\r') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'&' | b'*' if prev_is_boundary => {
                if let Some(&next) = bytes.get(i + 1) {
                    if !next.is_ascii_whitespace()
                        && !matches!(next, b',' | b']' | b'}' | b'[' | b'{')
                    {
                        return true;
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    false
}

/// Parsuje payload koperty według jej `format`: `json` bez zmian, `yaml` (string) -> wartość JSON.
pub fn parse_payload(format: &str, payload: &Value) -> Result<Value> {
    match format {
        formats::JSON => Ok(payload.clone()),
        formats::YAML => {
            let text = payload.as_str().context("yaml payload must be a string")?;
            if text.contains('\0') {
                bail!("yaml payload must not contain NUL bytes");
            }
            if has_yaml_anchor_or_alias(text) {
                bail!("yaml anchors and aliases are not allowed");
            }
            serde_yaml::from_str::<Value>(text).context("invalid yaml payload")
        }
        other => bail!("unsupported format `{other}`"),
    }
}

/// Usuwa bajty NUL (`\0`) ze stringa (Postgres TEXT/JSONB nie akceptuje `\u0000`).
pub(crate) fn strip_nuls_str(s: &mut String) {
    if s.contains('\0') {
        *s = s.replace('\0', "");
    }
}

/// Rekurencyjnie usuwa bajty NUL (`\0`) ze wszystkich kluczy i wartości stringowych w JSON.
pub(crate) fn strip_nuls_value(v: &mut Value) {
    match v {
        Value::String(s) => strip_nuls_str(s),
        Value::Array(arr) => {
            for item in arr {
                strip_nuls_value(item);
            }
        }
        Value::Object(map) => {
            if map.keys().any(|k| k.contains('\0')) {
                let old = std::mem::take(map);
                for (k, mut val) in old {
                    strip_nuls_value(&mut val);
                    map.insert(k.replace('\0', ""), val);
                }
            } else {
                for val in map.values_mut() {
                    strip_nuls_value(val);
                }
            }
        }
        _ => {}
    }
}

fn first<'a>(path: &JsonPath, raw: &'a Value) -> Option<&'a Value> {
    path.query(raw).first()
}

/// Wartość JSON jako tekst: string bez cudzysłowów, null/brak = None, reszta `to_string`.
fn as_text(v: Option<&Value>) -> Option<String> {
    match v {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(other) => Some(other.to_string()),
    }
}

/// FNV-1a 64 – stabilny między procesami (w odróżnieniu od `DefaultHasher`).
fn fnv1a64(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in data {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// FNV-1a 128 – używany do bezpiecznego skracania bardzo długich fingerprintów (limit indeksu B-tree).
fn fnv1a128(data: &[u8]) -> u128 {
    let mut hash: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    for &b in data {
        hash ^= u128::from(b);
        hash = hash.wrapping_mul(0x0000_0000_0100_0000_0000_0000_0000_013b);
    }
    hash
}

pub(crate) const MAX_FINGERPRINT_BYTES: usize = 512;

/// Ogranicza długość fingerprintu do `MAX_FINGERPRINT_BYTES` (zachowując prefiks UTF-8 + 128-bitowy hash).
pub(crate) fn bound_fingerprint(fp: String) -> String {
    if fp.len() <= MAX_FINGERPRINT_BYTES {
        return fp;
    }
    let hash = fnv1a128(fp.as_bytes());
    let mut end = 440.min(fp.len());
    while !fp.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}|h:{:032x}", &fp[..end], hash)
}

/// Escapuje `\` -> `\\` oraz `|` -> `\|` w segmencie fingerprintu, zapobiegając kolizjom separatora.
fn escape_fp_part(s: &str, out: &mut String) {
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '|' => out.push_str("\\|"),
            '\0' => {}
            c => out.push(c),
        }
    }
}

fn scale(name: &str) -> Option<Severity> {
    match name.to_ascii_lowercase().as_str() {
        "critical" | "crit" => Some(Severity::Critical),
        "error" => Some(Severity::Error),
        "warning" | "warn" => Some(Severity::Warning),
        "info" => Some(Severity::Info),
        _ => None,
    }
}

impl CompiledMapping {
    pub fn compile(def: MappingDef, version: i64) -> Result<Self> {
        let fields = def
            .fields
            .iter()
            .map(|(name, p)| Ok((name.clone(), parse_path(p)?)))
            .collect::<Result<Vec<_>>>()?;
        let title = match def.title.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(t) if t.contains("{{") => {
                condition::validate_template(t).context("invalid title template")?;
                Some(TitleSpec::Template(t.to_string()))
            }
            Some(t) => Some(TitleSpec::Path(parse_path(t)?)),
        };
        let severity = match &def.severity {
            SeveritySpec::Const { value } => SeverityCompiled::Const(*value),
            SeveritySpec::Path { path, map, default } => SeverityCompiled::Path {
                path: parse_path(path)?,
                map: map.clone(),
                default: *default,
            },
            SeveritySpec::Rules { rules, default } => SeverityCompiled::Rules {
                rules: rules
                    .iter()
                    .map(|r| Ok((condition::compile(&r.when)?, r.value)))
                    .collect::<Result<Vec<_>>>()?,
                default: *default,
            },
        };
        let fingerprint = def
            .fingerprint
            .iter()
            .map(|p| parse_path(p))
            .collect::<Result<Vec<_>>>()?;
        let resolved = def.resolved.as_ref().map(condition::compile).transpose()?;
        Ok(Self {
            def,
            version,
            fields,
            title,
            severity,
            fingerprint,
            resolved,
        })
    }

    /// Zamienia (już sparsowany) payload na zdarzenie kanoniczne. Brak pola = `null`, nie błąd.
    pub fn normalize(&self, env: &Envelope, raw: Value) -> CanonicalEvent {
        let mut fields = Map::new();
        for (name, path) in &self.fields {
            fields.insert(
                name.clone(),
                first(path, &raw).cloned().unwrap_or(Value::Null),
            );
        }
        let mut event = CanonicalEvent {
            event_id: env.event_id,
            schema: env.schema.clone(),
            source: env.source.clone(),
            severity: Severity::Warning,
            title: String::new(),
            fingerprint: String::new(),
            resolved: false,
            fields,
            raw,
            received_at: env.received_at,
            mapping_version: self.version,
        };
        let ctx = condition::event_context(&event);

        event.title = match &self.title {
            Some(TitleSpec::Template(t)) => condition::render_template(t, &ctx).unwrap_or_default(),
            Some(TitleSpec::Path(p)) => as_text(first(p, &event.raw)).unwrap_or_default(),
            None => String::new(),
        };
        if event.title.trim().is_empty() {
            event.title = env.schema.clone();
        }

        event.severity = match &self.severity {
            SeverityCompiled::Const(s) => *s,
            SeverityCompiled::Path { path, map, default } => as_text(first(path, &event.raw))
                .and_then(|v| {
                    map.get(&v)
                        .or_else(|| {
                            map.iter()
                                .find(|(k, _)| k.eq_ignore_ascii_case(&v))
                                .map(|(_, s)| s)
                        })
                        .copied()
                        .or_else(|| scale(&v))
                })
                .unwrap_or(*default),
            SeverityCompiled::Rules { rules, default } => rules
                .iter()
                .find(|(c, _)| c.evaluate(&ctx))
                .map(|(_, s)| *s)
                .unwrap_or(*default),
        };

        let fp = if self.fingerprint.is_empty() {
            let body = serde_json::to_string(&event.raw).unwrap_or_default();
            let mut fp = String::new();
            escape_fp_part(&env.schema, &mut fp);
            fp.push('|');
            fp.push_str(&format!("{:016x}", fnv1a64(body.as_bytes())));
            fp
        } else {
            let mut fp = String::new();
            escape_fp_part(&env.schema, &mut fp);
            for p in &self.fingerprint {
                fp.push('|');
                match as_text(first(p, &event.raw)) {
                    Some(v) => escape_fp_part(&v, &mut fp),
                    None => fp.push_str("\\N"),
                }
            }
            fp
        };
        event.fingerprint = bound_fingerprint(fp);

        event.resolved = self.resolved.as_ref().is_some_and(|c| c.evaluate(&ctx));

        strip_nuls_str(&mut event.schema);
        strip_nuls_str(&mut event.source);
        strip_nuls_str(&mut event.title);
        strip_nuls_str(&mut event.fingerprint);
        strip_nuls_value(&mut event.raw);
        for val in event.fields.values_mut() {
            strip_nuls_value(val);
        }
        event
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use uuid::Uuid;

    const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

    fn fixture(name: &str) -> contracts::IncomingEnvelope {
        let p = format!("{ROOT}/crates/contracts/fixtures/events/{name}.json");
        serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    fn seed_def(schema: &str) -> MappingDef {
        let p = format!("{ROOT}/config/mappings/{schema}.yaml");
        serde_yaml::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    fn env_of(i: &contracts::IncomingEnvelope) -> Envelope {
        Envelope {
            event_id: Uuid::new_v4(),
            format: i.format.clone(),
            schema: i.schema.clone(),
            payload: i.payload.clone(),
            source: "test".into(),
            received_at: chrono::Utc::now(),
        }
    }

    fn run(schema: &str, fixture_name: &str) -> CanonicalEvent {
        let m = CompiledMapping::compile(seed_def(schema), 1).unwrap();
        let env = env_of(&fixture(fixture_name));
        let raw = parse_payload(&env.format, &env.payload).unwrap();
        m.normalize(&env, raw)
    }

    #[test]
    fn workflow_fixture() {
        let ev = run("workflow.v1", "workflow_failed");
        assert_eq!(ev.severity, Severity::Error);
        assert_eq!(ev.title, "Counterparty service did not respond in 5000 ms");
        assert_eq!(
            ev.fingerprint,
            "workflow.v1|order-settlement|validate-counterparty|TIMEOUT"
        );
        assert_eq!(ev.raw, fixture("workflow_failed").payload);
        assert_eq!(ev.fields["workflowName"], "order-settlement");
        assert_eq!(ev.mapping_version, 1);
    }

    #[test]
    fn alertmanager_fixture() {
        let ev = run("alertmanager", "alertmanager_high_error_rate");
        assert_eq!(ev.fields["service"], "payments-api");
        assert_eq!(ev.severity, Severity::Critical);
        assert_eq!(ev.title, "Error rate 12% over the last 5 minutes");
        assert_eq!(
            ev.fingerprint,
            "alertmanager|HighErrorRate|payments-api|eu-west"
        );
    }

    #[test]
    fn trade_fixture_fingerprint() {
        let ev = run("trade.rejected.v2", "trade_rejected");
        assert_eq!(ev.fingerprint, "trade.rejected.v2|T-998877");
        assert_eq!(ev.fields["desk"], "rates");
        assert_eq!(ev.title, "trade.rejected.v2");
    }

    #[test]
    fn trade_fixture_severity_rules() {
        assert_eq!(
            run("trade.rejected.v2", "trade_rejected").severity,
            Severity::Critical
        );
    }

    #[test]
    fn alertmanager_resolved_condition() {
        let m = CompiledMapping::compile(seed_def("alertmanager"), 1).unwrap();
        let mut i = fixture("alertmanager_high_error_rate");
        i.payload["status"] = json!("resolved");
        let env = env_of(&i);
        assert!(m.normalize(&env, env.payload.clone()).resolved);
        let env = env_of(&fixture("alertmanager_high_error_rate"));
        assert!(!m.normalize(&env, env.payload.clone()).resolved);
    }

    #[test]
    fn missing_field_is_null() {
        let m = CompiledMapping::compile(seed_def("workflow.v1"), 1).unwrap();
        let mut i = fixture("workflow_failed");
        i.payload.as_object_mut().unwrap().remove("environment");
        let env = env_of(&i);
        let ev = m.normalize(&env, env.payload.clone());
        assert_eq!(ev.fields["environment"], Value::Null);
    }

    #[test]
    fn severity_scale_names() {
        let m = CompiledMapping::compile(seed_def("workflow.v1"), 1).unwrap();
        let mut i = fixture("workflow_failed");
        let sev = |m: &CompiledMapping, i: &contracts::IncomingEnvelope| {
            let env = env_of(i);
            m.normalize(&env, env.payload.clone()).severity
        };
        assert_eq!(sev(&m, &i), Severity::Error); // `ERROR` z mapy
        i.payload["severity"] = json!("WARN");
        assert_eq!(sev(&m, &i), Severity::Warning);
        i.payload["severity"] = json!("bogus");
        assert_eq!(sev(&m, &i), Severity::Warning); // default
        let am = CompiledMapping::compile(seed_def("alertmanager"), 1).unwrap();
        let mut a = fixture("alertmanager_high_error_rate");
        assert_eq!(sev(&am, &a), Severity::Critical);
        a.payload["labels"]["severity"] = json!("ERROR");
        assert_eq!(sev(&am, &a), Severity::Error);
    }

    #[test]
    fn same_fingerprint_for_different_workflow_ids() {
        let m = CompiledMapping::compile(seed_def("workflow.v1"), 1).unwrap();
        let fp = |id: &str| {
            let mut i = fixture("workflow_failed");
            i.payload["workflowId"] = json!(id);
            let env = env_of(&i);
            m.normalize(&env, env.payload.clone()).fingerprint
        };
        assert_eq!(fp("WF-1"), fp("WF-2"));
    }

    #[test]
    fn empty_fingerprint_uses_payload_hash() {
        let mut def = seed_def("workflow.v1");
        def.fingerprint.clear();
        let m = CompiledMapping::compile(def, 1).unwrap();
        let fp = |payload: Value| {
            let env = Envelope {
                payload: payload.clone(),
                ..env_of(&fixture("workflow_failed"))
            };
            m.normalize(&env, payload).fingerprint
        };
        let a = fp(json!({"a": 1}));
        assert_eq!(a, fp(json!({"a": 1})));
        assert_ne!(a, fp(json!({"a": 2})));
        let (schema, hash) = a.split_once('|').unwrap();
        assert_eq!(schema, "workflow.v1");
        assert_eq!(hash.len(), 16);
    }

    #[test]
    fn yaml_payload_is_parsed() {
        let raw = parse_payload("yaml", &json!("a: 1\nb:\n  c: x\n")).unwrap();
        assert_eq!(raw, json!({"a": 1, "b": {"c": "x"}}));
        assert!(parse_payload("yaml", &json!(5)).is_err());
        assert!(parse_payload("xml", &json!("<a/>")).is_err());

        let m = CompiledMapping::compile(seed_def("workflow.v1"), 1).unwrap();
        let yaml =
            "workflowName: wf\nstep: s\nseverity: INFO\nerror:\n  code: E1\n  message: boom\n";
        let env = Envelope {
            format: "yaml".into(),
            payload: json!(yaml),
            ..env_of(&fixture("workflow_failed"))
        };
        let ev = m.normalize(&env, parse_payload(&env.format, &env.payload).unwrap());
        assert_eq!(ev.fingerprint, "workflow.v1|wf|s|E1");
        assert_eq!(ev.title, "boom");
        assert_eq!(ev.severity, Severity::Info);
    }

    #[test]
    fn changed_title_path_recompiles() {
        let mut def = seed_def("workflow.v1");
        let env = env_of(&fixture("workflow_failed"));
        let t1 = CompiledMapping::compile(def.clone(), 1)
            .unwrap()
            .normalize(&env, env.payload.clone())
            .title;
        def.title = Some("$.workflowName".into());
        let m2 = CompiledMapping::compile(def, 2).unwrap();
        let ev = m2.normalize(&env, env.payload.clone());
        assert_ne!(t1, ev.title);
        assert_eq!(ev.title, "order-settlement");
        assert_eq!(ev.mapping_version, 2);
    }

    #[test]
    fn invalid_path_fails_compile() {
        let mut def = seed_def("workflow.v1");
        def.fingerprint.push("not a path".into());
        assert!(CompiledMapping::compile(def, 1).is_err());
    }

    #[test]
    fn fingerprint_escapes_delimiters_and_distinguishes_null_from_empty() {
        let m = CompiledMapping::compile(seed_def("workflow.v1"), 1).unwrap();
        let fp = |wf: Option<&str>, step: Option<&str>, code: Option<&str>| {
            let mut i = fixture("workflow_failed");
            match wf {
                Some(v) => i.payload["workflowName"] = json!(v),
                None => {
                    i.payload.as_object_mut().unwrap().remove("workflowName");
                }
            }
            match step {
                Some(v) => i.payload["step"] = json!(v),
                None => {
                    i.payload.as_object_mut().unwrap().remove("step");
                }
            }
            match code {
                Some(v) => i.payload["error"]["code"] = json!(v),
                None => {
                    i.payload["error"].as_object_mut().unwrap().remove("code");
                }
            }
            let env = env_of(&i);
            m.normalize(&env, env.payload.clone()).fingerprint
        };

        assert_ne!(
            fp(Some("A|B"), Some("C"), Some("D")),
            fp(Some("A"), Some("B|C"), Some("D"))
        );
        assert_ne!(
            fp(Some(""), Some("C"), Some("D")),
            fp(None, Some("C"), Some("D"))
        );
        assert_ne!(
            fp(Some("\\N"), Some("C"), Some("D")),
            fp(None, Some("C"), Some("D"))
        );
    }

    #[test]
    fn fingerprint_length_is_bounded_and_nuls_are_stripped() {
        let m = CompiledMapping::compile(seed_def("workflow.v1"), 1).unwrap();
        let mut i = fixture("workflow_failed");
        i.payload["workflowName"] = json!("a".repeat(1000));
        i.payload["error"]["message"] = json!("bad\0msg");
        let env = env_of(&i);
        let ev1 = m.normalize(&env, env.payload.clone());
        assert!(ev1.fingerprint.len() <= MAX_FINGERPRINT_BYTES);
        assert_eq!(ev1.title, "badmsg");
        assert!(!ev1.raw["error"]["message"].as_str().unwrap().contains('\0'));

        i.payload["workflowName"] = json!(format!("{}b", "a".repeat(999)));
        let env2 = env_of(&i);
        let ev2 = m.normalize(&env2, env2.payload.clone());
        assert!(ev2.fingerprint.len() <= MAX_FINGERPRINT_BYTES);
        assert_ne!(ev1.fingerprint, ev2.fingerprint);
    }

    #[test]
    fn yaml_payload_rejects_anchors_aliases_and_nul() {
        assert!(parse_payload("yaml", &json!("a: &a [1, 2]\nb: [*a, *a]\n")).is_err());
        assert!(parse_payload("yaml", &json!("a: 1\0\n")).is_err());
        assert!(parse_payload("yaml", &json!("msg: \"R&D *ok*\"\n")).is_ok());
    }
}
