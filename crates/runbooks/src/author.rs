//! `RunbookAuthor` – ekstrakcja szkicu runbooka AUTOMATIC z tekstu/Markdownu przez LLM (design D12).
//!
//! Jeden klient OpenAI-compatible (`/v1/chat/completions` ze Structured Outputs) obsługuje
//! zarówno `LLM_PROVIDER=local` (domyślnie Ollama), jak i `LLM_PROVIDER=api` (np. OpenAI/Gemini).
//! Domyślnie `LLM_PROVIDER=none` – brak wywołań LLM.
//!
//! Zasady bezpieczeństwa:
//! - przed wysłaniem do modelu treść przechodzi przez `redact::redact` (nigdy dane alertów),
//! - schemat JSON ogranicza `action` do enuma z bieżącego rejestru akcji,
//! - odpowiedź modelu zawsze przechodzi przez `Registry::classify` (whitelista akcji + `risk: high` ⇒ `approval: required`),
//! - wynik jest wyłącznie szkicem do akceptacji człowieka.

use std::collections::{BTreeMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use contracts::{ActionDef, Approval, Automation, Condition, RunbookDef, RunbookKind, StepDef, StepType};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::redact;
use crate::registry::Registry;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

const DEFAULT_LOCAL_URL: &str = "http://localhost:11434/v1";
const DEFAULT_LOCAL_MODEL: &str = "qwen2.5:7b";
const DEFAULT_API_URL: &str = "https://api.openai.com/v1";
const DEFAULT_API_MODEL: &str = "gpt-4o-mini";
const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const MAX_LLM_RESPONSE_BYTES: usize = 1024 * 1024;

/// Dane wejściowe do wygenerowania szkicu runbooka z tekstu.
#[derive(Debug, Clone)]
pub struct DraftRequest {
    pub id: Option<String>,
    pub title: Option<String>,
    pub text: String,
    pub matches: Option<Condition>,
    pub rule_ids: Vec<String>,
}

/// Wynik ekstrakcji gotowy do podglądu i akceptacji przez operatora.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftOutcome {
    pub draft: RunbookDef,
    pub yaml: String,
    pub redacted_count: usize,
    pub redacted_kinds: BTreeMap<String, usize>,
    pub warnings: Vec<String>,
}

/// Stan konfiguracji `RunbookAuthor` dla UI (`GET /runbooks/author/status`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorStatus {
    pub enabled: bool,
    pub provider: String,
    pub model: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum AuthorError {
    #[error("ekstrakcja LLM jest wyłączona (ustaw LLM_PROVIDER=local lub api)")]
    Disabled,
    #[error("niepoprawne dane wejściowe: {0}")]
    InvalidInput(String),
    #[error("przekroczono czas oczekiwania na odpowiedź LLM ({0} ms)")]
    Timeout(u64),
    #[error("błąd wywołania dostawcy LLM: {0}")]
    Upstream(String),
    #[error("niepoprawna odpowiedź modelu LLM: {0}")]
    InvalidResponse(String),
}

pub trait RunbookAuthor: Send + Sync {
    fn name(&self) -> &'static str;

    fn model(&self) -> Option<&str> {
        None
    }

    fn is_enabled(&self) -> bool {
        self.name() != "none"
    }

    fn status(&self) -> AuthorStatus {
        AuthorStatus {
            enabled: self.is_enabled(),
            provider: self.name().to_string(),
            model: self.model().map(str::to_string),
        }
    }

    fn draft<'a>(&'a self, req: &'a DraftRequest, actions: &'a [ActionDef]) -> BoxFuture<'a, Result<DraftOutcome, AuthorError>>;
}

/// Dostawca `none`: brak wywołań modelu.
pub struct NoAuthor;

impl RunbookAuthor for NoAuthor {
    fn name(&self) -> &'static str {
        "none"
    }

    fn draft<'a>(&'a self, _req: &'a DraftRequest, _actions: &'a [ActionDef]) -> BoxFuture<'a, Result<DraftOutcome, AuthorError>> {
        Box::pin(async { Err(AuthorError::Disabled) })
    }
}

/// Wspólny klient OpenAI-compatible (`/v1/chat/completions`) dla `local` (np. Ollama) i `api`.
pub struct OpenAiCompatAuthor {
    provider: &'static str,
    base_url: String,
    model: String,
    api_key: Option<String>,
    timeout_ms: u64,
    client: reqwest::Client,
}

impl OpenAiCompatAuthor {
    pub fn new(provider: &'static str, base_url: String, model: String, api_key: Option<String>, timeout_ms: u64) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder().timeout(Duration::from_millis(timeout_ms)).build()?;
        Ok(Self {
            provider,
            base_url: base_url.trim_end_matches('/').to_string(),
            model,
            api_key,
            timeout_ms,
            client,
        })
    }

    async fn call_chat_completions(&self, system_prompt: &str, user_prompt: &str, response_format: Value) -> Result<(u16, String), AuthorError> {
        let url = format!("{}/chat/completions", self.base_url);
        let payload = json!({
            "model": self.model,
            "temperature": 0.0,
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user", "content": user_prompt }
            ],
            "response_format": response_format
        });

        let mut req = self.client.post(&url).json(&payload);
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }

        let mut resp = req.send().await.map_err(|e| {
            if e.is_timeout() {
                AuthorError::Timeout(self.timeout_ms)
            } else {
                AuthorError::Upstream(e.without_url().to_string())
            }
        })?;
        let status = resp.status().as_u16();
        let mut buf = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(|e| {
            if e.is_timeout() {
                AuthorError::Timeout(self.timeout_ms)
            } else {
                AuthorError::Upstream(e.without_url().to_string())
            }
        })? {
            if buf.len() + chunk.len() > MAX_LLM_RESPONSE_BYTES {
                return Err(AuthorError::Upstream(format!(
                    "odpowiedź LLM przekracza limit {MAX_LLM_RESPONSE_BYTES} B"
                )));
            }
            buf.extend_from_slice(&chunk);
        }
        let body = String::from_utf8_lossy(&buf).into_owned();
        Ok((status, body))
    }

    async fn draft_inner(&self, req: &DraftRequest, actions: &[ActionDef]) -> Result<DraftOutcome, AuthorError> {
        if req.text.trim().is_empty() {
            return Err(AuthorError::InvalidInput("treść runbooka nie może być pusta".into()));
        }

        let redacted = redact::redact(&req.text);
        let registry = Registry::new(actions.to_vec());
        let mut all_actions: Vec<&ActionDef> = actions.iter().collect();
        let builtins = Registry::builtins();
        for b in &builtins {
            if !all_actions.iter().any(|a| a.name == b.name) {
                all_actions.push(b);
            }
        }
        all_actions.sort_by(|a, b| a.name.cmp(&b.name));

        let action_names: Vec<String> = all_actions.iter().map(|a| a.name.clone()).collect();
        let system_prompt = build_system_prompt(&all_actions);
        let user_prompt = format!(
            "Wyodrębnij kroki runbooka z poniższego tekstu:\n\n{}",
            redacted.text
        );
        let schema = build_response_schema(&action_names);

        let (mut status, mut body) = self
            .call_chat_completions(
                &system_prompt,
                &user_prompt,
                json!({
                    "type": "json_schema",
                    "json_schema": {
                        "name": "runbook_draft",
                        "strict": true,
                        "schema": schema
                    }
                }),
            )
            .await?;

        // Fallback dla serwerów OpenAI-compatible bez obsługi `json_schema`.
        if status == 400 && (body.contains("response_format") || body.contains("json_schema")) {
            let (s2, b2) = self
                .call_chat_completions(&system_prompt, &user_prompt, json!({ "type": "json_object" }))
                .await?;
            status = s2;
            body = b2;
        }

        if !(200..300).contains(&status) {
            tracing::warn!(status, provider = self.provider, "LLM upstream returned non-2xx status");
            return Err(AuthorError::Upstream(format!("HTTP {status}")));
        }

        let content = extract_message_content(&body)?;
        let cleaned = strip_code_fences(&content);
        let parsed: LlmDraftPayload = serde_json::from_str(cleaned)
            .map_err(|e| AuthorError::InvalidResponse(format!("niepoprawny JSON szkicu: {e}")))?;

        let (draft, warnings) = build_and_classify_draft(req, parsed, &registry)?;
        let yaml = render_runbook_yaml(&draft);

        Ok(DraftOutcome {
            draft,
            yaml,
            redacted_count: redacted.count,
            redacted_kinds: redacted.kinds,
            warnings,
        })
    }
}

impl RunbookAuthor for OpenAiCompatAuthor {
    fn name(&self) -> &'static str {
        self.provider
    }

    fn model(&self) -> Option<&str> {
        Some(&self.model)
    }

    fn draft<'a>(&'a self, req: &'a DraftRequest, actions: &'a [ActionDef]) -> BoxFuture<'a, Result<DraftOutcome, AuthorError>> {
        Box::pin(self.draft_inner(req, actions))
    }
}

/// Dostawca z `LLM_PROVIDER` (domyślnie `none`). Nieznana nazwa lub brak wymaganego klucza = błąd startu.
pub fn from_env() -> anyhow::Result<Arc<dyn RunbookAuthor>> {
    let provider = std::env::var("LLM_PROVIDER").unwrap_or_else(|_| "none".to_string());
    let timeout_ms = std::env::var("LLM_TIMEOUT_MS")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.parse::<u64>())
        .transpose()
        .map_err(|e| anyhow::anyhow!("niepoprawna wartość LLM_TIMEOUT_MS: {e}"))?
        .unwrap_or(DEFAULT_TIMEOUT_MS);

    match provider.trim() {
        "" | "none" => Ok(Arc::new(NoAuthor)),
        "local" => {
            let base_url = env_non_empty("LLM_BASE_URL").unwrap_or_else(|| DEFAULT_LOCAL_URL.to_string());
            let model = env_non_empty("LLM_MODEL").unwrap_or_else(|| DEFAULT_LOCAL_MODEL.to_string());
            let api_key = env_non_empty("LLM_API_KEY");
            Ok(Arc::new(OpenAiCompatAuthor::new("local", base_url, model, api_key, timeout_ms)?))
        }
        "api" => {
            let base_url = env_non_empty("LLM_BASE_URL").unwrap_or_else(|| DEFAULT_API_URL.to_string());
            let model = env_non_empty("LLM_MODEL").unwrap_or_else(|| DEFAULT_API_MODEL.to_string());
            let api_key = env_non_empty("LLM_API_KEY")
                .ok_or_else(|| anyhow::anyhow!("LLM_PROVIDER=api wymaga ustawienia zmiennej LLM_API_KEY"))?;
            Ok(Arc::new(OpenAiCompatAuthor::new("api", base_url, model, Some(api_key), timeout_ms)?))
        }
        other => anyhow::bail!("nieobsługiwany LLM_PROVIDER={other} (dozwolone: none, local, api)"),
    }
}

fn env_non_empty(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

fn build_system_prompt(actions: &[&ActionDef]) -> String {
    let mut catalog = String::new();
    for a in actions {
        let desc = redact::redact(a.description.as_deref().unwrap_or("brak opisu")).text;
        let schema_raw = a.params_schema.as_ref().map(|v| v.to_string()).unwrap_or_else(|| "{}".into());
        let schema = redact::redact(&schema_raw).text;
        catalog.push_str(&format!("- `{}` (risk: {:?}): {} | params_schema: {}\n", a.name, a.risk, desc, schema));
    }
    format!(
        "Jesteś asystentem przekształcającym opis runbooka (Markdown lub wolny tekst) w listę kroków JSON.\n\
         Zasady:\n\
         1. Używaj WYŁĄCZNIE akcji z poniższego rejestru. Jeśli krok nie pasuje do żadnej akcji, ustaw type=\"manual\" i action=null.\n\
         2. Warunki słowne (np. 'jeśli ponowienie zawiedzie dwa razy') zapisz w polu `condition` jako tekst i ustaw type=\"manual\" lub `approval: \"required\"`.\n\
         3. Zachowuj zmienne szablonowe `{{{{...}}}}` dokładnie w oryginalnej postaci.\n\
         4. Nie wymyślaj poleceń shell ani nowych nazw akcji.\n\
         5. Zwróć wyłącznie poprawny obiekt JSON zgodny ze schematem.\n\n\
         Dostępny rejestr akcji:\n{catalog}"
    )
}

fn build_response_schema(action_names: &[String]) -> Value {
    let mut action_enum: Vec<Value> = action_names.iter().map(|n| Value::String(n.clone())).collect();
    action_enum.push(Value::Null);

    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["title", "steps"],
        "properties": {
            "title": { "type": "string" },
            "steps": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["id", "type", "text", "action", "params", "approval", "condition"],
                    "properties": {
                        "id": { "type": "string" },
                        "type": { "type": "string", "enum": ["check", "action", "manual", "note"] },
                        "text": { "type": "string" },
                        "action": { "type": ["string", "null"], "enum": action_enum },
                        "params": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "additionalProperties": false,
                                "required": ["key", "value"],
                                "properties": {
                                    "key": { "type": "string" },
                                    "value": { "type": "string" }
                                }
                            }
                        },
                        "approval": { "type": "string", "enum": ["none", "required"] },
                        "condition": { "type": ["string", "null"] }
                    }
                }
            }
        }
    })
}

fn extract_message_content(body: &str) -> Result<String, AuthorError> {
    let v: Value = serde_json::from_str(body)
        .map_err(|e| AuthorError::InvalidResponse(format!("odpowiedź HTTP nie jest JSON-em: {e}")))?;
    v.pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| AuthorError::InvalidResponse("brak pola choices[0].message.content".into()))
}

fn strip_code_fences(raw: &str) -> &str {
    let trimmed = raw.trim();
    let Some(rest) = trimmed.strip_prefix("```") else { return trimmed };
    let rest = rest.strip_prefix("json").or_else(|| rest.strip_prefix("JSON")).unwrap_or(rest);
    let rest = rest.trim_start_matches(['\r', '\n']);
    rest.strip_suffix("```").unwrap_or(rest).trim()
}

#[derive(Debug, Deserialize)]
struct LlmDraftPayload {
    #[serde(default)]
    title: String,
    #[serde(default)]
    steps: Vec<LlmStep>,
}

#[derive(Debug, Deserialize)]
struct LlmStep {
    #[serde(default)]
    id: String,
    #[serde(rename = "type", default = "default_step_type")]
    step_type: StepType,
    #[serde(default)]
    text: String,
    #[serde(default)]
    action: Option<String>,
    #[serde(default)]
    params: LlmParams,
    #[serde(default)]
    approval: Approval,
    #[serde(default)]
    condition: Option<String>,
}

fn default_step_type() -> StepType {
    StepType::Manual
}

/// Akceptuje zarówno tablicę par `[{key, value}]` (strict JSON Schema), jak i mapę `{k: v}`.
#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
enum LlmParams {
    #[default]
    Empty,
    Pairs(Vec<ParamPair>),
    Map(Map<String, Value>),
}

#[derive(Debug, Deserialize)]
struct ParamPair {
    key: String,
    value: Value,
}

impl LlmParams {
    fn into_map(self) -> Map<String, Value> {
        match self {
            LlmParams::Empty => Map::new(),
            LlmParams::Map(m) => m,
            LlmParams::Pairs(pairs) => pairs
                .into_iter()
                .filter(|p| !p.key.trim().is_empty())
                .map(|p| (p.key, p.value))
                .collect(),
        }
    }
}

fn build_and_classify_draft(
    req: &DraftRequest,
    payload: LlmDraftPayload,
    registry: &Registry,
) -> Result<(RunbookDef, Vec<String>), AuthorError> {
    if payload.steps.is_empty() {
        return Err(AuthorError::InvalidResponse("model nie wyodrębnił żadnych kroków".into()));
    }

    let title = req
        .title
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            let t = payload.title.trim();
            (!t.is_empty()).then(|| t.to_string())
        })
        .unwrap_or_else(|| "Extracted runbook".to_string());

    let id = req
        .id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| slugify(&title));

    let mut seen_ids = HashSet::new();
    let mut raw_steps = Vec::with_capacity(payload.steps.len());
    for (idx, step) in payload.steps.into_iter().enumerate() {
        let mut step_id = step.id.trim().to_string();
        if step_id.is_empty() || !seen_ids.insert(step_id.clone()) {
            let mut n = idx + 1;
            loop {
                let candidate = format!("s{n}");
                if seen_ids.insert(candidate.clone()) {
                    step_id = candidate;
                    break;
                }
                n += 1;
            }
        }
        let action = step.action.map(|a| a.trim().to_string()).filter(|a| !a.is_empty());
        let condition = step.condition.map(|c| c.trim().to_string()).filter(|c| !c.is_empty());
        let text = if step.text.trim().is_empty() {
            action.clone().unwrap_or_else(|| format!("Krok {}", idx + 1))
        } else {
            step.text.trim().to_string()
        };
        raw_steps.push(StepDef {
            id: step_id,
            step_type: step.step_type,
            text,
            action,
            params: step.params.into_map(),
            approval: step.approval,
            condition,
            automation: Automation::Manual,
        });
    }

    let before = raw_steps.clone();
    let draft = RunbookDef {
        id,
        title,
        kind: RunbookKind::Automatic,
        matches: req.matches.clone(),
        rule_ids: req.rule_ids.clone(),
        body: None,
        steps: raw_steps,
        source_file: None,
        version: 0,
    };
    let mut resolved = registry.classify(&draft);

    let mut warnings = Vec::new();
    for (orig, cls) in before.iter().zip(resolved.steps.iter_mut()) {
        if let Some(action_name) = &cls.action {
            if cls.automation == Automation::Manual {
                warnings.push(format!(
                    "Krok {}: akcja `{action_name}` nie istnieje w rejestrze — zdegradowano do kroku ręcznego.",
                    cls.id
                ));
            }
            if orig.approval == Approval::None && cls.approval == Approval::Required {
                warnings.push(format!(
                    "Krok {}: akcja `{action_name}` ma risk=high — wymuszono zatwierdzenie.",
                    cls.id
                ));
            }
            if action_name == "http_get" {
                cls.approval = Approval::Required;
                warnings.push(format!(
                    "Krok {}: wygenerowano akcję `http_get` — wymuszono zatwierdzenie i zweryfikuj parametr `url` przed zapisaniem.",
                    cls.id
                ));
            }
        }

        let known_params: Option<HashSet<String>> = cls
            .action
            .as_deref()
            .and_then(|name| registry.get(name))
            .and_then(|a| a.params_schema.as_ref())
            .and_then(|schema| {
                let mut set = HashSet::new();
                if let Some(props) = schema.get("properties").and_then(Value::as_object) {
                    set.extend(props.keys().cloned());
                }
                if let Some(req_keys) = schema.get("required").and_then(Value::as_array) {
                    set.extend(req_keys.iter().filter_map(Value::as_str).map(str::to_string));
                }
                (!set.is_empty()).then_some(set)
            });

        for (key, val) in &cls.params {
            if let Some(ref allowed_keys) = known_params {
                if !allowed_keys.contains(key) {
                    warnings.push(format!(
                        "Krok {}: niezweryfikowany parametr `{key}` spoza schematu akcji — sprawdź przed zapisaniem.",
                        cls.id
                    ));
                }
            }
            validate_param_templates(&cls.id, key, val, &mut warnings)?;
        }
    }

    Ok((resolved, warnings))
}

fn validate_param_templates(step_id: &str, key: &str, val: &Value, warnings: &mut Vec<String>) -> Result<(), AuthorError> {
    match val {
        Value::String(s) => {
            condition::validate_template(s).map_err(|e| {
                AuthorError::InvalidResponse(format!("krok {step_id}: niepoprawny szablon w parametrze `{key}`: {e}"))
            })?;
            if s.contains("[REDACTED_") {
                warnings.push(format!(
                    "Krok {step_id}: parametr `{key}` zawiera zredagowany placeholder — uzupełnij wartość przed zapisaniem."
                ));
            }
        }
        Value::Array(items) => {
            for item in items {
                validate_param_templates(step_id, key, item, warnings)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn slugify(input: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in input.chars() {
        let c = ch.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() { "extracted-runbook".into() } else { out }
}

/// Renderuje `RunbookDef` do czytelnego YAML zgodnego z `crate::parse::parse_runbook`.
pub fn render_runbook_yaml(def: &RunbookDef) -> String {
    let mut doc = serde_yaml::Mapping::new();
    doc.insert("runbook".into(), def.id.clone().into());
    doc.insert("title".into(), def.title.clone().into());
    if let Some(matches) = &def.matches {
        if let Ok(v) = serde_yaml::to_value(matches) {
            doc.insert("matches".into(), v);
        }
    }
    if !def.rule_ids.is_empty() {
        if let Ok(v) = serde_yaml::to_value(&def.rule_ids) {
            doc.insert("rule_ids".into(), v);
        }
    }
    let steps: Vec<serde_yaml::Value> = def
        .steps
        .iter()
        .map(|s| {
            let mut m = serde_yaml::Mapping::new();
            m.insert("id".into(), s.id.clone().into());
            let step_type = match s.step_type {
                StepType::Check => "check",
                StepType::Action => "action",
                StepType::Manual => "manual",
                StepType::Note => "note",
            };
            m.insert("type".into(), step_type.into());
            m.insert("text".into(), s.text.clone().into());
            if let Some(action) = &s.action {
                m.insert("action".into(), action.clone().into());
            }
            if !s.params.is_empty() {
                if let Ok(v) = serde_yaml::to_value(&s.params) {
                    m.insert("params".into(), v);
                }
            }
            if s.approval == Approval::Required {
                m.insert("approval".into(), "required".into());
            }
            if let Some(cond) = &s.condition {
                m.insert("if".into(), cond.clone().into());
            }
            serde_yaml::Value::Mapping(m)
        })
        .collect();
    doc.insert("steps".into(), serde_yaml::Value::Sequence(steps));
    serde_yaml::to_string(&serde_yaml::Value::Mapping(doc)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
    use contracts::{ActionKind, HttpActionSpec, Risk};

    use super::*;

    fn sample_actions() -> Vec<ActionDef> {
        vec![ActionDef {
            name: "restart_service".into(),
            kind: ActionKind::Http,
            risk: Risk::High,
            description: Some("Restartuje usługę".into()),
            params_schema: Some(json!({"type": "object", "required": ["service"]})),
            http: Some(HttpActionSpec {
                method: "POST".into(),
                url: "http://mock-targets:8090/restart/{{service}}".into(),
                headers: Default::default(),
                body: None,
                timeout_ms: 5000,
            }),
            version: 1,
        }]
    }

    async fn spawn_mock_llm(status: StatusCode, body: Value) -> (String, Arc<Mutex<Vec<Value>>>) {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let cap = captured.clone();
        let app = Router::new()
            .route(
                "/v1/chat/completions",
                post(move |State(c): State<Arc<Mutex<Vec<Value>>>>, Json(req): Json<Value>| {
                    let body = body.clone();
                    async move {
                        c.lock().unwrap().push(req);
                        (status, Json(body))
                    }
                }),
            )
            .with_state(cap);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{addr}/v1"), captured)
    }

    #[tokio::test]
    async fn none_provider_returns_disabled() {
        let author = NoAuthor;
        assert!(!author.is_enabled());
        assert_eq!(author.name(), "none");
        let req = DraftRequest {
            id: None,
            title: None,
            text: "# Test\n1. Do something".into(),
            matches: None,
            rule_ids: vec![],
        };
        let err = author.draft(&req, &[]).await.unwrap_err();
        assert!(matches!(err, AuthorError::Disabled));
    }

    #[tokio::test]
    async fn extracts_redacts_and_classifies_steps() {
        let llm_content = json!({
            "title": "Payments error recovery",
            "steps": [
                {
                    "id": "s1",
                    "type": "action",
                    "text": "Restart {{raw.service}}",
                    "action": "restart_service",
                    "params": [{"key": "service", "value": "{{raw.service}}"}],
                    "approval": "none",
                    "condition": null
                },
                {
                    "id": "s2",
                    "type": "action",
                    "text": "Drop database",
                    "action": "drop_database",
                    "params": [],
                    "approval": "none",
                    "condition": null
                }
            ]
        });
        let (base_url, captured) = spawn_mock_llm(
            StatusCode::OK,
            json!({
                "choices": [{ "message": { "content": llm_content.to_string() } }]
            }),
        )
        .await;

        let author = OpenAiCompatAuthor::new("local", base_url, "qwen2.5:7b".into(), None, 5000).unwrap();
        let req = DraftRequest {
            id: Some("payments-recovery".into()),
            title: None,
            text: "Check http://secret.corp:8080/health on 10.0.0.5 and notify ops@bank.internal for {{raw.service}}".into(),
            matches: None,
            rule_ids: vec!["payments-prod".into()],
        };
        let outcome = author.draft(&req, &sample_actions()).await.unwrap();

        // Sprawdzamy, że do mock serwera poszedł wyłącznie zredagowany tekst.
        let sent = captured.lock().unwrap();
        assert_eq!(sent.len(), 1);
        let user_msg = sent[0]["messages"][1]["content"].as_str().unwrap();
        assert!(!user_msg.contains("secret.corp"));
        assert!(!user_msg.contains("10.0.0.5"));
        assert!(!user_msg.contains("ops@bank.internal"));
        assert!(user_msg.contains("{{raw.service}}"));
        assert_eq!(outcome.redacted_count, 3);

        // Krok 1: risk=high ⇒ wymuszone approval=required, automation=automated.
        let s1 = &outcome.draft.steps[0];
        assert_eq!(s1.automation, Automation::Automated);
        assert_eq!(s1.approval, Approval::Required);
        assert_eq!(s1.params.get("service").and_then(Value::as_str), Some("{{raw.service}}"));

        // Krok 2: halucynowana akcja ⇒ degradacja do manual + ostrzeżenie.
        let s2 = &outcome.draft.steps[1];
        assert_eq!(s2.automation, Automation::Manual);
        assert_eq!(outcome.warnings.len(), 2);

        // Wyrenderowany YAML parsuje się poprawnie przez `parse::parse_runbook`.
        let reparsed = crate::parse::parse_runbook("draft.yaml", &outcome.yaml).unwrap();
        assert_eq!(reparsed.id, "payments-recovery");
        assert_eq!(reparsed.steps.len(), 2);
    }

    #[tokio::test]
    async fn handles_upstream_error_and_invalid_json() {
        let (base_url, _) = spawn_mock_llm(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": "boom"})).await;
        let author = OpenAiCompatAuthor::new("local", base_url, "qwen2.5:7b".into(), None, 5000).unwrap();
        let req = DraftRequest { id: None, title: None, text: "Check service".into(), matches: None, rule_ids: vec![] };
        let err = author.draft(&req, &[]).await.unwrap_err();
        assert!(matches!(err, AuthorError::Upstream(_)));
        assert!(!err.to_string().contains("boom"), "raw upstream error body should not be reflected: {err}");

        let (bad_url, _) = spawn_mock_llm(
            StatusCode::OK,
            json!({ "choices": [{ "message": { "content": "not json at all" } }] }),
        )
        .await;
        let author2 = OpenAiCompatAuthor::new("local", bad_url, "qwen2.5:7b".into(), None, 5000).unwrap();
        let err2 = author2.draft(&req, &[]).await.unwrap_err();
        assert!(matches!(err2, AuthorError::InvalidResponse(_)));
    }

    #[tokio::test]
    async fn redacts_system_prompt_and_caps_oversized_llm_response() {
        let mut actions = sample_actions();
        actions[0].description = Some("Calls https://secret.corp/api with Bearer tok_secret123".into());
        actions[0].params_schema = Some(json!({
            "type": "object",
            "required": ["service"],
            "properties": { "service": { "type": "string", "default": "db-01.prod.internal" } }
        }));

        let llm_content = json!({
            "title": "Health check",
            "steps": [{
                "id": "s1",
                "type": "check",
                "text": "Check health",
                "action": "http_get",
                "params": [
                    {"key": "url", "value": "[REDACTED_URL_1]"},
                    {"key": "extra_param", "value": "val"}
                ],
                "approval": "none",
                "condition": null
            }]
        });
        let (base_url, captured) = spawn_mock_llm(
            StatusCode::OK,
            json!({ "choices": [{ "message": { "content": llm_content.to_string() } }] }),
        )
        .await;
        let author = OpenAiCompatAuthor::new("local", base_url, "qwen2.5:7b".into(), None, 5000).unwrap();
        let req = DraftRequest { id: None, title: None, text: "Check service".into(), matches: None, rule_ids: vec![] };
        let outcome = author.draft(&req, &actions).await.unwrap();

        let sent = captured.lock().unwrap();
        let sys_msg = sent[0]["messages"][0]["content"].as_str().unwrap();
        assert!(!sys_msg.contains("secret.corp"));
        assert!(!sys_msg.contains("tok_secret123"));
        assert!(!sys_msg.contains("db-01.prod.internal"));

        // LLM-drafted http_get requires approval and emits warnings for http_get, unverified param, and redacted placeholder.
        assert_eq!(outcome.draft.steps[0].approval, Approval::Required);
        assert!(outcome.warnings.iter().any(|w| w.contains("http_get")));
        assert!(outcome.warnings.iter().any(|w| w.contains("extra_param")));
        assert!(outcome.warnings.iter().any(|w| w.contains("placeholder")));

        // Oversized LLM response (> 1 MiB) is rejected.
        let huge = "x".repeat(MAX_LLM_RESPONSE_BYTES + 1024);
        let (huge_url, _) = spawn_mock_llm(StatusCode::OK, json!({ "padding": huge })).await;
        let huge_author = OpenAiCompatAuthor::new("local", huge_url, "qwen2.5:7b".into(), None, 5000).unwrap();
        let err = huge_author.draft(&req, &[]).await.unwrap_err();
        assert!(matches!(err, AuthorError::Upstream(_)));
    }

    #[test]
    fn rejects_invalid_template_in_llm_generated_params() {
        let registry = Registry::new(sample_actions());
        let req = DraftRequest { id: None, title: None, text: "test".into(), matches: None, rule_ids: vec![] };
        let payload = LlmDraftPayload {
            title: "Bad".into(),
            steps: vec![LlmStep {
                id: "s1".into(),
                step_type: StepType::Action,
                text: "Restart".into(),
                action: Some("restart_service".into()),
                params: LlmParams::Pairs(vec![ParamPair {
                    key: "service".into(),
                    value: json!("{{env.DATABASE_URL}}"),
                }]),
                approval: Approval::None,
                condition: None,
            }],
        };
        let err = build_and_classify_draft(&req, payload, &registry).unwrap_err();
        assert!(matches!(err, AuthorError::InvalidResponse(_)));
    }
}
