//! Rejestr akcji i wykonanie akcji HTTP (zadania 10.1–10.3, design D11).
//!
//! Wymuszenia niezależne od treści runbooka:
//! - wykonuje się tylko akcja z rejestru (reszta = krok manualny),
//! - `risk: high` ⇒ krok zawsze wymaga zatwierdzenia,
//! - host z URL musi być na allowliście, także po przekierowaniu,
//! - timeout i limit rozmiaru odpowiedzi,
//! - sekrety tylko jako `${ENV}` w nagłówkach, rozwijane w chwili wysłania.

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use contracts::{ActionDef, ActionKind, Approval, Automation, Risk, RunbookDef, StepDef};
use reqwest::redirect;
use reqwest::Url;
use serde_json::{json, Map, Value};

/// Limit odpowiedzi akcji HTTP; więcej = błąd kroku.
pub const MAX_RESPONSE_BYTES: usize = 256 * 1024;
/// Timeout `http_get` (akcje deklaratywne mają własny `timeout_ms`).
const HTTP_GET_TIMEOUT_MS: u64 = 5000;
const MAX_TIMEOUT_MS: u64 = 30_000;
const MAX_REDIRECTS: usize = 5;
const METHODS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];

fn builtins() -> Vec<ActionDef> {
    let builtin = |name: &str, description: &str, params_schema: Value| ActionDef {
        name: name.into(),
        kind: ActionKind::Builtin,
        risk: Risk::Low,
        description: Some(description.into()),
        params_schema: Some(params_schema),
        http: None,
        version: 0,
    };
    vec![
        builtin(
            "http_get",
            "GET na adres z parametru `url` (np. health check); sukces = odpowiedź 2xx",
            json!({"type": "object", "required": ["url"], "properties": {"url": {"type": "string"}}}),
        ),
        builtin(
            "escalate",
            "Powiadamia członków zespołu z parametru `team` zgodnie z ich preferencjami",
            json!({"type": "object", "required": ["team"], "properties": {"team": {"type": "string"}}}),
        ),
        builtin(
            "notify",
            "Powiadamia osoby z parametru `people` zgodnie z ich preferencjami",
            json!({"type": "object", "required": ["people"], "properties": {"people": {"type": "array", "items": {"type": "string"}}}}),
        ),
    ]
}

/// Sprawdza akcję z pliku albo z API, zanim trafi do rejestru.
pub fn validate_action(def: &ActionDef) -> Result<(), String> {
    if def.name.is_empty() || !def.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(format!("niepoprawna nazwa akcji `{}` (dozwolone: litery, cyfry, _ i -)", def.name));
    }
    if builtins().iter().any(|b| b.name == def.name) {
        return Err(format!("`{}` to akcja wbudowana; wybierz inną nazwę", def.name));
    }
    if def.kind != ActionKind::Http {
        return Err("w konfiguracji można definiować tylko akcje `kind: http`".into());
    }
    let http = def.http.as_ref().ok_or("brak sekcji `http`")?;
    if !METHODS.contains(&http.method.as_str()) {
        return Err(format!("metoda `{}` niedozwolona (dozwolone: {})", http.method, METHODS.join(", ")));
    }
    if !(1..=MAX_TIMEOUT_MS).contains(&http.timeout_ms) {
        return Err(format!("timeout_ms musi być w zakresie 1..={MAX_TIMEOUT_MS}"));
    }
    condition::validate_template(&http.url).map_err(|e| format!("url: {e}"))?;
    if let Some(body) = &http.body {
        condition::validate_template(body).map_err(|e| format!("body: {e}"))?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub struct Registry {
    actions: HashMap<String, ActionDef>,
}

impl Default for Registry {
    fn default() -> Self {
        Self::new(vec![])
    }
}

impl Registry {
    /// Akcje wbudowane + deklaratywne. Wbudowanych nie da się nadpisać.
    pub fn new(custom: Vec<ActionDef>) -> Self {
        let mut actions: HashMap<_, _> = custom.into_iter().map(|a| (a.name.clone(), a)).collect();
        for b in builtins() {
            actions.insert(b.name.clone(), b);
        }
        Self { actions }
    }

    pub fn get(&self, name: &str) -> Option<&ActionDef> {
        self.actions.get(name)
    }

    pub fn len(&self) -> usize {
        self.actions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    /// Ustala, które kroki wykonają się automatycznie, i wymusza zatwierdzenie dla `risk: high`.
    /// Wywoływane przy każdym przeładowaniu, więc dodanie brakującej akcji „naprawia” runbooki.
    pub fn classify(&self, def: &RunbookDef) -> RunbookDef {
        let mut def = def.clone();
        self.classify_steps(&mut def.steps);
        def
    }

    fn classify_steps(&self, steps: &mut [StepDef]) {
        for step in steps.iter_mut() {
            step.automation = match step.action.as_deref().and_then(|a| self.get(a)) {
                Some(_) => Automation::Automated,
                None => Automation::Manual,
            };
        }
        self.enforce_approval(steps);
    }

    /// `risk: high` ⇒ zatwierdzenie. Maszyna stanów woła to ponownie przed wykonaniem,
    /// bo ryzyko akcji mogło wzrosnąć po starcie runa (bez ruszania `automation`,
    /// które mógł zmienić tryb ręczny).
    pub fn enforce_approval(&self, steps: &mut [StepDef]) {
        for step in steps {
            if step.action.as_deref().and_then(|a| self.get(a)).is_some_and(|a| a.risk == Risk::High) {
                step.approval = Approval::Required;
            }
        }
    }

    /// Akcje wbudowane (zapisywane do tabeli `actions`, żeby api pokazywało pełny rejestr).
    pub fn builtins() -> Vec<ActionDef> {
        builtins()
    }
}

/// Akcje użyte w krokach, których nie ma w rejestrze (po `classify`).
pub fn unhandled_actions(def: &RunbookDef) -> impl Iterator<Item = &str> {
    def.steps.iter().filter(|s| s.automation == Automation::Manual).filter_map(|s| s.action.as_deref())
}

#[derive(Debug, thiserror::Error)]
pub enum ActionError {
    #[error("host niedozwolony: {0}")]
    HostNotAllowed(String),
    #[error("przekierowanie na host niedozwolony albo za dużo przekierowań")]
    RedirectNotAllowed,
    #[error("niepoprawny URL `{0}`")]
    InvalidUrl(String),
    #[error("brak parametru `{0}`")]
    MissingParam(&'static str),
    #[error("brak sekretu `${{{0}}}` w środowisku")]
    MissingSecret(String),
    #[error("szablon: {0}")]
    Template(String),
    #[error("timeout po {0} ms")]
    Timeout(u64),
    #[error("odpowiedź większa niż {MAX_RESPONSE_BYTES} B")]
    ResponseTooLarge,
    #[error("odpowiedź HTTP {status}: {body}")]
    Status { status: u16, body: String },
    #[error("żądanie nieudane: {0}")]
    Request(String),
    #[error("akcja `{0}` nie jest akcją HTTP")]
    NotHttp(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActionOutput {
    pub status: u16,
    pub body: String,
}

/// Wykonuje akcje HTTP (`http_get` i deklaratywne). `escalate`/`notify` publikują
/// `deliveries.requested` i wykonuje je maszyna stanów (faza B).
#[derive(Clone)]
pub struct Executor {
    client: reqwest::Client,
    allowlist: Vec<String>,
}

impl Executor {
    pub fn new(allowlist: Vec<String>) -> Self {
        let allowed = allowlist.clone();
        let policy = redirect::Policy::custom(move |attempt| {
            let ok = attempt.previous().len() < MAX_REDIRECTS && host_allowed(&allowed, attempt.url());
            if ok {
                attempt.follow()
            } else {
                attempt.error(ActionError::RedirectNotAllowed)
            }
        });
        let client = reqwest::Client::builder().redirect(policy).build().expect("reqwest client");
        Self { client, allowlist }
    }

    /// `params` – parametry kroku (już z podstawionymi wartościami alertu),
    /// `alert_ctx` – `condition::event_context`; szablony URL/body widzą oba, parametry wygrywają.
    pub async fn execute(
        &self,
        action: &ActionDef,
        params: &Map<String, Value>,
        alert_ctx: &Value,
        idempotency_key: &str,
    ) -> Result<ActionOutput, ActionError> {
        let (method, url, headers, body, timeout_ms) = match (action.kind, action.http.as_ref()) {
            (ActionKind::Builtin, _) if action.name == "http_get" => {
                let url = params.get("url").and_then(Value::as_str).ok_or(ActionError::MissingParam("url"))?;
                ("GET".to_string(), url.to_string(), BTreeMap::new(), None, HTTP_GET_TIMEOUT_MS)
            }
            (ActionKind::Http, Some(http)) => {
                let ctx = overlay(alert_ctx, params);
                let render = |t: &str| condition::render_template(t, &ctx).map_err(|e| ActionError::Template(e.to_string()));
                let body = http.body.as_deref().map(render).transpose()?;
                (http.method.clone(), render(&http.url)?, http.headers.clone(), body, http.timeout_ms)
            }
            _ => return Err(ActionError::NotHttp(action.name.clone())),
        };

        let url = Url::parse(&url).map_err(|_| ActionError::InvalidUrl(url.clone()))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(ActionError::InvalidUrl(url.to_string()));
        }
        if !host_allowed(&self.allowlist, &url) {
            return Err(ActionError::HostNotAllowed(url.host_str().unwrap_or_default().to_string()));
        }
        let method = reqwest::Method::from_bytes(method.as_bytes()).map_err(|_| ActionError::Request(format!("metoda {method}")))?;
        let mut req = self
            .client
            .request(method, url)
            .timeout(Duration::from_millis(timeout_ms))
            .header("Idempotency-Key", idempotency_key);
        for (name, value) in &headers {
            req = req.header(name, expand_secrets(value)?);
        }
        if let Some(body) = body {
            req = req.header("Content-Type", "application/json").body(body);
        }

        let map_err = |e: reqwest::Error| {
            if e.is_timeout() {
                ActionError::Timeout(timeout_ms)
            } else if e.is_redirect() {
                ActionError::RedirectNotAllowed
            } else {
                ActionError::Request(e.without_url().to_string())
            }
        };
        let mut resp = req.send().await.map_err(map_err)?;
        let status = resp.status().as_u16();
        let mut buf = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(map_err)? {
            if buf.len() + chunk.len() > MAX_RESPONSE_BYTES {
                return Err(ActionError::ResponseTooLarge);
            }
            buf.extend_from_slice(&chunk);
        }
        let body = String::from_utf8_lossy(&buf).into_owned();
        if !(200..300).contains(&status) {
            return Err(ActionError::Status { status, body });
        }
        Ok(ActionOutput { status, body })
    }
}

fn host_allowed(allowlist: &[String], url: &Url) -> bool {
    url.host_str().is_some_and(|h| allowlist.iter().any(|a| a.eq_ignore_ascii_case(h)))
}

fn overlay(alert_ctx: &Value, params: &Map<String, Value>) -> Value {
    let mut ctx = alert_ctx.as_object().cloned().unwrap_or_default();
    ctx.extend(params.clone());
    Value::Object(ctx)
}

/// `Bearer ${TOKEN}` → wartość z env. Sekret nie trafia do definicji, dziennika ani błędów.
fn expand_secrets(value: &str) -> Result<String, ActionError> {
    static SECRET_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = SECRET_RE.get_or_init(|| {
        regex::Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}").expect("valid regex")
    });
    let mut missing = None;
    let out = re.replace_all(value, |c: &regex::Captures| {
        std::env::var(&c[1]).unwrap_or_else(|_| {
            missing = Some(c[1].to_string());
            String::new()
        })
    });
    match missing {
        Some(name) => Err(ActionError::MissingSecret(name)),
        None => Ok(out.into_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_runbook;
    use axum::{http::HeaderMap, response::Redirect, routing::get, Router};
    use contracts::HttpActionSpec;

    fn http_action(name: &str, risk: Risk, url: &str, timeout_ms: u64) -> ActionDef {
        ActionDef {
            name: name.into(),
            kind: ActionKind::Http,
            risk,
            description: None,
            params_schema: None,
            http: Some(HttpActionSpec { method: "POST".into(), url: url.into(), headers: BTreeMap::new(), body: None, timeout_ms }),
            version: 1,
        }
    }

    /// Lokalna usługa docelowa: echo nagłówków, wolna odpowiedź, duża odpowiedź, przekierowanie.
    async fn spawn_target() -> String {
        let app = Router::new()
            .route("/ok", get(|| async { "ok" }).post(|h: HeaderMap| async move {
                let get = |n: &str| h.get(n).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
                format!("{}|{}", get("idempotency-key"), get("authorization"))
            }))
            .route("/slow", get(|| async {
                tokio::time::sleep(Duration::from_secs(10)).await;
                "late"
            }))
            .route("/big", get(|| async { "x".repeat(MAX_RESPONSE_BYTES + 1) }))
            .route("/fail", get(|| async { (axum::http::StatusCode::SERVICE_UNAVAILABLE, "down") }))
            .route("/redirect", get(|| async { Redirect::temporary("http://evil.example/steal") }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    fn executor() -> Executor {
        Executor::new(vec!["127.0.0.1".into()])
    }

    async fn http_get(url: &str) -> Result<ActionOutput, ActionError> {
        let registry = Registry::default();
        let params = json!({ "url": url }).as_object().unwrap().clone();
        executor().execute(registry.get("http_get").unwrap(), &params, &json!({}), "k").await
    }

    #[tokio::test]
    async fn http_get_ok_and_failure() {
        let base = spawn_target().await;
        assert_eq!(http_get(&format!("{base}/ok")).await.unwrap().body, "ok");
        assert!(matches!(http_get(&format!("{base}/fail")).await, Err(ActionError::Status { status: 503, .. })));
    }

    #[tokio::test]
    async fn host_outside_allowlist_is_rejected() {
        let err = http_get("http://counterparty-service.evil/health").await.unwrap_err();
        assert!(matches!(err, ActionError::HostNotAllowed(ref h) if h == "counterparty-service.evil"), "{err}");
    }

    #[tokio::test]
    async fn redirect_to_other_host_is_rejected() {
        let base = spawn_target().await;
        let err = http_get(&format!("{base}/redirect")).await.unwrap_err();
        assert!(matches!(err, ActionError::RedirectNotAllowed), "{err}");
    }

    #[tokio::test]
    async fn slow_service_times_out() {
        let base = spawn_target().await;
        let mut action = http_action("slow", Risk::Low, &format!("{base}/slow"), 200);
        action.http.as_mut().unwrap().method = "GET".into();
        let started = std::time::Instant::now();
        let err = executor().execute(&action, &Map::new(), &json!({}), "k").await.unwrap_err();
        assert!(matches!(err, ActionError::Timeout(200)), "{err}");
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[tokio::test]
    async fn oversized_response_fails() {
        let base = spawn_target().await;
        assert!(matches!(http_get(&format!("{base}/big")).await, Err(ActionError::ResponseTooLarge)));
    }

    #[tokio::test]
    async fn secret_and_idempotency_key_are_sent() {
        let base = spawn_target().await;
        let mut action = http_action("open_ticket", Risk::Low, &format!("{base}/ok"), 1000);
        action.http.as_mut().unwrap().headers.insert("Authorization".into(), "Bearer ${P3_TEST_TICKET_TOKEN}".into());
        std::env::set_var("P3_TEST_TICKET_TOKEN", "s3cret");
        let out = executor().execute(&action, &Map::new(), &json!({}), "step:r:s2:0").await.unwrap();
        assert_eq!(out.body, "step:r:s2:0|Bearer s3cret");

        action.http.as_mut().unwrap().headers.insert("Authorization".into(), "Bearer ${P3_TEST_MISSING}".into());
        let err = executor().execute(&action, &Map::new(), &json!({}), "k").await.unwrap_err();
        assert!(matches!(err, ActionError::MissingSecret(ref n) if n == "P3_TEST_MISSING"), "{err}");
    }

    #[test]
    fn high_risk_forces_approval_and_unknown_action_is_manual() {
        let def = parse_runbook(
            "r.yaml",
            "steps:\n  - action: restart_service\n    approval: none\n  - action: flush_cdn\n  - action: shell\n    command: rm -rf /\n",
        )
        .unwrap();
        let registry = Registry::new(vec![http_action("restart_service", Risk::High, "http://mock-targets:8090/restart_service", 5000)]);
        let resolved = registry.classify(&def);
        assert_eq!((resolved.steps[0].automation, resolved.steps[0].approval), (Automation::Automated, Approval::Required));
        assert_eq!(resolved.steps[1].automation, Automation::Manual);
        assert_eq!(unhandled_actions(&resolved).collect::<Vec<_>>(), ["flush_cdn", "shell"]);

        // Dodanie brakującej akcji: przy kolejnym przeładowaniu krok jest automatyczny.
        let registry = Registry::new(vec![http_action("flush_cdn", Risk::Low, "http://mock-targets:8090/cdn/flush", 5000)]);
        assert_eq!(registry.classify(&def).steps[1].automation, Automation::Automated);
    }

    #[test]
    fn builtins_cannot_be_redefined() {
        assert!(validate_action(&http_action("escalate", Risk::Low, "http://x/", 1000)).unwrap_err().contains("wbudowana"));
        assert!(validate_action(&http_action("flush_cache", Risk::High, "http://x/", 1000)).is_ok());
    }
}
