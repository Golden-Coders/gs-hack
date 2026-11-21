//! Redakcja danych wrażliwych przed wysłaniem opisu runbooka do LLM (design D12).
//!
//! Zamienia adresy URL, e-maile, adresy IPv4 oraz wewnętrzne nazwy hostów na deterministyczne
//! placeholdery, zachowując zmienne szablonowe `{{...}}` bez zmian.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use regex::Regex;

/// Wynik redakcji treści runbooka.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactedText {
    pub text: String,
    pub count: usize,
    pub kinds: BTreeMap<String, usize>,
}

fn template_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\{\{[^{}]*\}\}").expect("valid template regex"))
}

fn url_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"https?://[^\s)"'<>\]}]+"#).expect("valid url regex"))
}

fn email_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b").expect("valid email regex"))
}

fn ipv4_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\b(?:(?:25[0-5]|2[0-4]\d|1?\d?\d)\.){3}(?:25[0-5]|2[0-4]\d|1?\d?\d)(?::\d{1,5})?\b")
            .expect("valid ipv4 regex")
    })
}

fn internal_host_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\b[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?(?:\.[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?)*\.(?:cluster\.local|internal|corp|local|svc|lan)(?::\d{1,5})?\b")
            .expect("valid host regex")
    })
}

fn replace_kind(input: &str, re: &Regex, kind: &str, label: &str, kinds: &mut BTreeMap<String, usize>, total: &mut usize) -> String {
    let mut seq = 0usize;
    let out = re
        .replace_all(input, |_: &regex::Captures| {
            seq += 1;
            *total += 1;
            *kinds.entry(kind.to_string()).or_insert(0) += 1;
            format!("[REDACTED_{label}_{seq}]")
        })
        .into_owned();
    out
}

/// Redaguje wrażliwe dane (URL, e-mail, IPv4, wewnętrzne hosty) z zachowaniem `{{...}}`.
pub fn redact(input: &str) -> RedactedText {
    // 1. Chronimy zmienne szablonowe `{{...}}`, aby nie zostały naruszone przez redakcję.
    let mut templates: Vec<String> = Vec::new();
    let protected = template_re()
        .replace_all(input, |caps: &regex::Captures| {
            let idx = templates.len();
            templates.push(caps[0].to_string());
            format!("\x00TPL_{idx}\x00")
        })
        .into_owned();

    // 2. Redagujemy kolejno: URL -> email -> IPv4 -> wewnętrzny host.
    let mut kinds = BTreeMap::new();
    let mut count = 0usize;
    let step = replace_kind(&protected, url_re(), "url", "URL", &mut kinds, &mut count);
    let step = replace_kind(&step, email_re(), "email", "EMAIL", &mut kinds, &mut count);
    let step = replace_kind(&step, ipv4_re(), "ip", "IP", &mut kinds, &mut count);
    let mut step = replace_kind(&step, internal_host_re(), "host", "HOST", &mut kinds, &mut count);

    // 3. Przywracamy zmienne szablonowe.
    for (idx, tpl) in templates.into_iter().enumerate() {
        step = step.replace(&format!("\x00TPL_{idx}\x00"), &tpl);
    }

    RedactedText { text: step, count, kinds }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_url_ip_email_and_internal_host_preserving_templates() {
        let input = "Check http://internal-pay.corp:8080/health on 10.20.30.40, ping db-01.prod.internal:5432, and notify ops@bank.internal for {{raw.workflowId}}.";
        let res = redact(input);

        assert!(!res.text.contains("http://internal-pay.corp:8080/health"));
        assert!(!res.text.contains("10.20.30.40"));
        assert!(!res.text.contains("db-01.prod.internal"));
        assert!(!res.text.contains("ops@bank.internal"));
        assert!(res.text.contains("[REDACTED_URL_1]"));
        assert!(res.text.contains("[REDACTED_IP_1]"));
        assert!(res.text.contains("[REDACTED_HOST_1]"));
        assert!(res.text.contains("[REDACTED_EMAIL_1]"));
        assert!(res.text.contains("{{raw.workflowId}}"));
        assert_eq!(res.count, 4);
        assert_eq!(res.kinds.get("url"), Some(&1));
        assert_eq!(res.kinds.get("ip"), Some(&1));
        assert_eq!(res.kinds.get("host"), Some(&1));
        assert_eq!(res.kinds.get("email"), Some(&1));
    }

    #[test]
    fn preserves_service_names_and_plain_text() {
        let input = "Restart payments-api and check order-settlement latency for {{raw.service}}.";
        let res = redact(input);
        assert_eq!(res.text, input);
        assert_eq!(res.count, 0);
        assert!(res.kinds.is_empty());
    }

    #[test]
    fn preserves_templates_containing_host_like_paths() {
        let input = "Call {{raw.service.internal}} and email oncall@example.com";
        let res = redact(input);
        assert!(res.text.contains("{{raw.service.internal}}"));
        assert!(res.text.contains("[REDACTED_EMAIL_1]"));
        assert_eq!(res.count, 1);
    }
}
