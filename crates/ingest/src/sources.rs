//! Źródła Kafki z `$CONFIG_DIR/p1/sources/*.yaml`: skaner plików + konsument na źródło.

use std::{collections::HashMap, path::Path, sync::Arc, time::Duration};

use common::{
    db::sqlx::PgPool,
    kafka::{
        self,
        rdkafka::{
            admin::{AdminClient, AdminOptions, NewTopic, TopicReplication},
            client::DefaultClientContext,
            config::ClientConfig,
            message::Message,
            producer::FutureProducer,
            types::RDKafkaErrorCode,
        },
    },
};
use serde::Deserialize;
use serde_json::Value;
use tokio::task::JoinHandle;

use crate::{accept, parse_payload, record_error, ParseError};

/// Definicja źródła (lokalna dla ingest, nie jest kontraktem).
#[derive(Debug, Clone, Deserialize)]
pub struct SourceDef {
    pub topic: String,
    pub schema: String,
    pub format: String,
}

pub struct Ctx {
    pub brokers: String,
    pub pool: PgPool,
    pub producer: FutureProducer,
}

/// Skanuje katalog co 2 s: nowy plik -> start, zmieniona treść -> restart, usunięty -> stop.
/// Polling zamiast `notify`: bind-mount Dockera na macOS nie przekazuje zdarzeń systemu plików.
pub async fn run_scanner(ctx: Arc<Ctx>, dir: String) {
    let mut running: HashMap<String, (String, JoinHandle<()>)> = HashMap::new();
    loop {
        let found = read_dir(Path::new(&dir));
        running.retain(|name, (_, h)| {
            let keep = found.contains_key(name);
            if !keep {
                tracing::info!(source = %name, "source removed, stopping");
                h.abort();
            }
            keep
        });
        for (name, text) in found {
            if running.get(&name).is_some_and(|(old, _)| *old == text) {
                continue;
            }
            let def: SourceDef = match serde_yaml::from_str(&text) {
                Ok(d) => d,
                Err(e) => {
                    tracing::warn!(source = %name, error = %e, "invalid source file, skipping");
                    continue;
                }
            };
            if let Some((_, old)) = running.remove(&name) {
                tracing::info!(source = %name, "source changed, restarting");
                old.abort();
            }
            tracing::info!(source = %name, topic = %def.topic, "starting source");
            running.insert(name.clone(), (text, tokio::spawn(run_source(ctx.clone(), name, def))));
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

fn read_dir(dir: &Path) -> HashMap<String, String> {
    let Ok(entries) = std::fs::read_dir(dir) else { return HashMap::new() };
    entries
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let ext = p.extension()?.to_str()?;
            if ext != "yaml" && ext != "yml" {
                return None;
            }
            Some((p.file_stem()?.to_str()?.to_string(), std::fs::read_to_string(&p).ok()?))
        })
        .collect()
}

async fn run_source(ctx: Arc<Ctx>, name: String, def: SourceDef) {
    let group = format!("ingest-source-{name}");
    if let Err(e) = ensure_topic(&ctx.brokers, &def.topic).await {
        tracing::warn!(source = %name, topic = %def.topic, error = %e, "ensuring source topic failed");
    }
    let consumer = match kafka::consumer(&ctx.brokers, &group, &[def.topic.as_str()]) {
        Ok(c) => c,
        Err(e) => return tracing::error!(source = %name, error = %e, "creating source consumer failed"),
    };
    loop {
        match consumer.recv().await {
            Ok(msg) => {
                handle(&ctx, &def, msg.payload().unwrap_or_default()).await;
                if let Err(e) = kafka::commit(&consumer, &msg) {
                    tracing::warn!(source = %name, error = %e, "commit failed");
                }
            }
            Err(e) => {
                tracing::warn!(source = %name, error = %e, "kafka receive error");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

/// Tworzy topik źródła, jeśli nie istnieje (Redpanda w compose nie tworzy topików automatycznie,
/// a `topics-init` zakłada tylko topiki wewnętrzne). Istniejący topik zostaje bez zmian.
/// shortcut: replikacja 1 (jeden broker w compose), przy klastrze produkcyjnym topiki źródeł zakłada ich właściciel.
pub async fn ensure_topic(brokers: &str, topic: &str) -> anyhow::Result<()> {
    let admin: AdminClient<DefaultClientContext> = ClientConfig::new().set("bootstrap.servers", brokers).create()?;
    let new = NewTopic::new(topic, contracts::topics::PARTITIONS, TopicReplication::Fixed(1));
    for result in admin.create_topics(&[new], &AdminOptions::new()).await? {
        match result {
            Ok(_) | Err((_, RDKafkaErrorCode::TopicAlreadyExists)) => {}
            Err((t, code)) => anyhow::bail!("creating topic {t}: {code}"),
        }
    }
    Ok(())
}

/// Buduje (format, schema, payload) z wiadomości: gotowa koperta albo dane wg konfiguracji źródła.
pub fn build(def: &SourceDef, bytes: &[u8]) -> Result<(String, String, Value), ParseError> {
    let json: Option<Value> = serde_json::from_slice(bytes).ok();
    if let Some(Value::Object(o)) = &json {
        if let (Some(Value::String(f)), Some(Value::String(s)), Some(p)) = (o.get("format"), o.get("schema"), o.get("payload")) {
            return Ok((f.clone(), s.clone(), parse_payload(f, p)?));
        }
    }
    let payload = match def.format.as_str() {
        "json" => json.ok_or_else(|| ParseError::Invalid("invalid json".into()))?,
        "yaml" => Value::String(String::from_utf8(bytes.to_vec()).map_err(|e| ParseError::Invalid(format!("not utf-8: {e}")))?),
        other => return Err(ParseError::Unsupported(other.to_string())),
    };
    Ok((def.format.clone(), def.schema.clone(), parse_payload(&def.format, &payload)?))
}

async fn handle(ctx: &Ctx, def: &SourceDef, bytes: &[u8]) {
    let source = format!("kafka:{}", def.topic);
    match build(def, bytes) {
        Ok((format, schema, payload)) => {
            // Ponawiamy do skutku: offset zostanie zatwierdzony dopiero po publikacji.
            while let Err(e) = accept(&ctx.producer, &format, &schema, payload.clone(), source.clone()).await {
                tracing::warn!(error = %e, "publishing to events.raw failed, retrying");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
        Err(ParseError::Unsupported(f)) => record_error(&ctx.pool, &ctx.producer, &source, &format!("unsupported format: {f}"), bytes).await,
        Err(ParseError::Invalid(d)) => record_error(&ctx.pool, &ctx.producer, &source, &d, bytes).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn def(format: &str) -> SourceDef {
        SourceDef { topic: "t".into(), schema: "src.v1".into(), format: format.into() }
    }

    #[test]
    fn envelope_message_passes_unchanged() {
        let msg = br#"{"format":"json","schema":"other.v2","payload":{"x":1}}"#;
        let (f, s, p) = build(&def("json"), msg).unwrap();
        assert_eq!((f.as_str(), s.as_str(), p), ("json", "other.v2", json!({"x": 1})));
    }

    #[test]
    fn raw_json_gets_source_schema_and_format() {
        let (f, s, p) = build(&def("json"), br#"{"x":1}"#).unwrap();
        assert_eq!((f.as_str(), s.as_str(), p), ("json", "src.v1", json!({"x": 1})));
    }

    #[test]
    fn raw_yaml_becomes_string() {
        let (f, _, p) = build(&def("yaml"), b"a: 1\n").unwrap();
        assert_eq!((f.as_str(), p), ("yaml", json!("a: 1\n")));
    }

    #[test]
    fn invalid_json_is_error() {
        assert!(matches!(build(&def("json"), b"{not json"), Err(ParseError::Invalid(_))));
    }

    #[test]
    fn unsupported_source_format() {
        assert!(matches!(build(&def("xml"), b"<a/>"), Err(ParseError::Unsupported(_))));
    }
}
