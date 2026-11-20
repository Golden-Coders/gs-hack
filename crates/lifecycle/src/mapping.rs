//! Mapping schematu -> zdarzenie kanoniczne (czysta logika, bez IO).

use std::collections::BTreeMap;

use anyhow::{bail, Context, Result};
use contracts::{formats, CanonicalEvent, Envelope, MappingDef, Severity, SeveritySpec};
use serde_json::{Map, Value};
use serde_json_path::JsonPath;

enum SeverityCompiled {
    Const(Severity),
    Path { path: JsonPath, map: BTreeMap<String, Severity>, default: Severity },
    Rules { rules: Vec<(condition::Compiled, Severity)>, default: Severity },
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

/// Parsuje payload koperty według jej `format`: `json` bez zmian, `yaml` (string) -> wartość JSON.
pub fn parse_payload(format: &str, payload: &Value) -> Result<Value> {
    match format {
        formats::JSON => Ok(payload.clone()),
        formats::YAML => {
            let text = payload.as_str().context("yaml payload must be a string")?;
            serde_yaml::from_str::<Value>(text).context("invalid yaml payload")
        }
        other => bail!("unsupported format `{other}`"),
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
            SeveritySpec::Path { path, map, default } => {
                SeverityCompiled::Path { path: parse_path(path)?, map: map.clone(), default: *default }
            }
            SeveritySpec::Rules { rules, default } => SeverityCompiled::Rules {
                rules: rules
                    .iter()
                    .map(|r| Ok((condition::compile(&r.when)?, r.value)))
                    .collect::<Result<Vec<_>>>()?,
                default: *default,
            },
        };
        let fingerprint = def.fingerprint.iter().map(|p| parse_path(p)).collect::<Result<Vec<_>>>()?;
        let resolved = def.resolved.as_ref().map(condition::compile).transpose()?;
        Ok(Self { def, version, fields, title, severity, fingerprint, resolved })
    }

    /// Zamienia (już sparsowany) payload na zdarzenie kanoniczne. Brak pola = `null`, nie błąd.
    pub fn normalize(&self, env: &Envelope, raw: Value) -> CanonicalEvent {
        let mut fields = Map::new();
        for (name, path) in &self.fields {
            fields.insert(name.clone(), first(path, &raw).cloned().unwrap_or(Value::Null));
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
        // Kontekst dla warunków i szablonów: pola kanoniczne + `raw.*` (zob. condition::event_context).
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
                        .or_else(|| map.iter().find(|(k, _)| k.eq_ignore_ascii_case(&v)).map(|(_, s)| s))
                        .copied()
                        .or_else(|| scale(&v))
                })
                .unwrap_or(*default),
            SeverityCompiled::Rules { rules, default } => {
                rules.iter().find(|(c, _)| c.evaluate(&ctx)).map(|(_, s)| *s).unwrap_or(*default)
            }
        };

        event.fingerprint = if self.fingerprint.is_empty() {
            let body = serde_json::to_string(&event.raw).unwrap_or_default();
            format!("{}|{:016x}", env.schema, fnv1a64(body.as_bytes()))
        } else {
            let mut fp = env.schema.clone();
            for p in &self.fingerprint {
                fp.push('|');
                fp.push_str(&as_text(first(p, &event.raw)).unwrap_or_default());
            }
            fp
        };

        event.resolved = self.resolved.as_ref().is_some_and(|c| c.evaluate(&ctx));
        event
    }
}
