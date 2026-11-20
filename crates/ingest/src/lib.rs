//! ingest – przyjmowanie zdarzeń (HTTP + źródła Kafki) i publikacja na `events.raw`.

pub mod http;
pub mod sources;

use chrono::Utc;
use common::kafka::{publish, rdkafka::producer::FutureProducer};
use contracts::{topics, Envelope, UiEvent, UiEventKind};
use serde_json::{json, Value};
use uuid::Uuid;

#[derive(Debug, PartialEq)]
pub enum ParseError {
    /// Nieobsługiwany format (HTTP 415).
    Unsupported(String),
    /// Payload niezgodny z formatem (HTTP 400).
    Invalid(String),
}

/// Jedno miejsce parsowania formatów; zwraca payload gotowy do wysłania w kopercie.
/// `json` – dowolna wartość; `yaml` – string z poprawnym YAML (zwracany bez zmian).
pub fn parse_payload(format: &str, payload: &Value) -> Result<Value, ParseError> {
    match format {
        "json" => Ok(payload.clone()),
        "yaml" => {
            let text = payload.as_str().ok_or_else(|| ParseError::Invalid("yaml payload must be a string".into()))?;
            serde_yaml::from_str::<serde_yaml::Value>(text).map_err(|e| ParseError::Invalid(format!("invalid yaml: {e}")))?;
            Ok(payload.clone())
        }
        other => Err(ParseError::Unsupported(other.to_string())),
    }
}

/// Nadaje identyfikator i czas, publikuje na `events.raw` (czeka na ack brokera).
pub async fn accept(producer: &FutureProducer, format: &str, schema: &str, payload: Value, source: String) -> anyhow::Result<Uuid> {
    let envelope = Envelope {
        event_id: Uuid::now_v7(),
        format: format.to_string(),
        schema: schema.to_string(),
        payload,
        source,
        received_at: Utc::now(),
    };
    publish(producer, topics::EVENTS_RAW, &envelope.event_id.to_string(), &envelope).await?;
    Ok(envelope.event_id)
}

/// Zapisuje błąd ingestu w bazie i powiadamia UI (`ui.events`).
pub async fn record_error(pool: &common::db::sqlx::PgPool, producer: &FutureProducer, source: &str, error: &str, sample: &[u8]) {
    let sample: String = String::from_utf8_lossy(sample).chars().take(1000).collect();
    if let Err(e) = common::db::sqlx::query("INSERT INTO ingest_errors (source, error, sample) VALUES ($1, $2, $3)")
        .bind(source)
        .bind(error)
        .bind(sample)
        .execute(pool)
        .await
    {
        tracing::warn!(error = %e, "saving ingest error failed");
    }
    let ev = UiEvent { alert_id: None, kind: UiEventKind::IngestError, payload: json!({"source": source, "error": error}), at: Utc::now() };
    if let Err(e) = publish(producer, topics::UI_EVENTS, "ingest", &ev).await {
        tracing::warn!(error = %e, "publishing ingest error to ui.events failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_json_unchanged() {
        let v = json!({"a": [1, 2]});
        assert_eq!(parse_payload("json", &v), Ok(v));
    }

    #[test]
    fn parse_yaml_string_ok() {
        let v = Value::String("a: 1\nb: [x, y]\n".into());
        assert_eq!(parse_payload("yaml", &v), Ok(v));
    }

    #[test]
    fn parse_yaml_invalid() {
        let v = Value::String("a: [unclosed".into());
        assert!(matches!(parse_payload("yaml", &v), Err(ParseError::Invalid(_))));
    }

    #[test]
    fn parse_yaml_non_string() {
        assert!(matches!(parse_payload("yaml", &json!({"a": 1})), Err(ParseError::Invalid(_))));
    }

    #[test]
    fn parse_unsupported() {
        assert_eq!(parse_payload("xml", &json!("<a/>")), Err(ParseError::Unsupported("xml".into())));
    }
}
