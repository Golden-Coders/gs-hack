//! Źródła Kafki z `$CONFIG_DIR/sources/*.yaml`: skaner plików + konsument na źródło.

use std::{collections::HashMap, path::Path, sync::Arc, time::Duration};

use common::{
    db::sqlx::PgPool,
    kafka::{
        concurrent_consumer, run_concurrent, ConcurrentConfig, LaneHandler,
        rdkafka::{
            admin::{AdminClient, AdminOptions, NewTopic, TopicReplication},
            client::DefaultClientContext,
            config::ClientConfig,
            message::{Message, OwnedMessage},
            producer::FutureProducer,
            types::RDKafkaErrorCode,
        },
    },
};
use serde::Deserialize;
use serde_json::Value;
use tokio::{sync::watch, task::JoinHandle};

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

/// Działające źródło: treść pliku, sygnał stop i zadanie konsumenta.
struct Running {
    text: String,
    stop: watch::Sender<bool>,
    handle: JoinHandle<()>,
}

/// Limit czekania na zakończenie źródła (drenowanie pasów + commit) przed `abort()`.
/// Musi być większy od `drain_timeout` konsumenta (domyślnie 5 s).
const STOP_TIMEOUT: Duration = Duration::from_secs(15);

/// Zatrzymuje źródło łagodnie: sygnał stop, drenowanie i commit offsetów, a po `STOP_TIMEOUT` twardy `abort()`.
/// Niezcommitowane wiadomości wrócą do nowego konsumenta (at-least-once; skutki idempotentne po `event_id`
/// są po stronie dalszych etapów, ewentualny duplikat na `events.raw` jest dopuszczalny).
async fn stop_source(name: &str, mut r: Running) {
    let _ = r.stop.send(true);
    if tokio::time::timeout(STOP_TIMEOUT, &mut r.handle).await.is_err() {
        tracing::warn!(source = %name, "source did not stop in time, aborting");
        r.handle.abort();
    }
}

/// Skanuje katalog co 2 s: nowy plik -> start, zmieniona treść -> restart, usunięty -> stop.
/// Polling zamiast `notify`: bind-mount Dockera na macOS nie przekazuje zdarzeń systemu plików.
pub async fn run_scanner(ctx: Arc<Ctx>, dir: String) {
    let mut running: HashMap<String, Running> = HashMap::new();
    loop {
        let found = read_dir(Path::new(&dir));
        let removed: Vec<String> = running.keys().filter(|n| !found.contains_key(*n)).cloned().collect();
        for name in removed {
            if let Some(r) = running.remove(&name) {
                tracing::info!(source = %name, "source removed, stopping");
                stop_source(&name, r).await;
            }
        }
        for (name, text) in found {
            if running.get(&name).is_some_and(|r| r.text == text) {
                continue;
            }
            let def: SourceDef = match serde_yaml::from_str(&text) {
                Ok(d) => d,
                Err(e) => {
                    tracing::warn!(source = %name, error = %e, "invalid source file, skipping");
                    continue;
                }
            };
            if let Some(old) = running.remove(&name) {
                tracing::info!(source = %name, "source changed, restarting");
                stop_source(&name, old).await;
            }
            tracing::info!(source = %name, topic = %def.topic, "starting source");
            let (stop, rx) = watch::channel(false);
            let handle = tokio::spawn(run_source(ctx.clone(), name.clone(), def, rx));
            running.insert(name, Running { text, stop, handle });
        }
        tokio::time::sleep(common::reload::POLL_INTERVAL).await;
    }
}

fn read_dir(dir: &Path) -> HashMap<String, String> {
    common::reload::yaml_files(dir)
        .into_iter()
        .filter_map(|p| {
            Some((p.file_stem()?.to_str()?.to_string(), std::fs::read_to_string(&p).ok()?))
        })
        .collect()
}

/// Domyślna liczba pasów konsumenta źródła (nadpisanie: `INGEST_SOURCE_LANES`).
const DEFAULT_LANES: usize = 32;

/// Konsument źródła: współbieżnie (pasy per klucz wiadomości źródłowej), commit offsetu po publikacji
/// na `events.raw`. Kończy się po sygnale `shutdown` (drenowanie + commit) albo błędzie utworzenia konsumenta.
pub async fn run_source(ctx: Arc<Ctx>, name: String, def: SourceDef, shutdown: watch::Receiver<bool>) {
    let group = format!("ingest-source-{name}");
    if let Err(e) = ensure_topic(&ctx.brokers, &def.topic).await {
        tracing::warn!(source = %name, topic = %def.topic, error = %e, "ensuring source topic failed");
    }
    let consumer = match concurrent_consumer(&ctx.brokers, &group, &[def.topic.as_str()]) {
        Ok(c) => c,
        Err(e) => return tracing::error!(source = %name, error = %e, "creating source consumer failed"),
    };
    let mut config = ConcurrentConfig::from_env("INGEST_SOURCE", DEFAULT_LANES);
    // Drenowanie musi się zmieścić w `STOP_TIMEOUT` (po nim `stop_source` robi twardy `abort`).
    let max_drain = STOP_TIMEOUT.saturating_sub(Duration::from_secs(5));
    if config.drain_timeout > max_drain {
        tracing::warn!(source = %name, requested = ?config.drain_timeout, ?max_drain, "drain timeout exceeds stop budget; clamping");
        config.drain_timeout = max_drain;
    }
    let handler = SourceHandler { ctx, def };
    if let Err(e) = run_concurrent(consumer, config, shutdown, move |_| handler.clone()).await {
        tracing::error!(source = %name, error = %e, "source consumer stopped with error");
    }
}

/// Bezstanowy handler pasa: `build` + `accept` albo `record_error`.
#[derive(Clone)]
struct SourceHandler {
    ctx: Arc<Ctx>,
    def: SourceDef,
}

impl LaneHandler for SourceHandler {
    async fn handle(&mut self, msg: &OwnedMessage) -> anyhow::Result<()> {
        handle(&self.ctx, &self.def, msg.payload().unwrap_or_default()).await
    }
}

/// Tworzy topik źródła, jeśli nie istnieje (Redpanda w compose nie tworzy topików automatycznie;
/// `topics-init` zakłada też `trades`, ale nowe źródła dodane w locie obsługuje ta funkcja).
/// Istniejący topik zostaje bez zmian (z ostrzeżeniem, gdy ma za mało partycji).
/// shortcut: replikacja 1 (jeden broker w compose), przy klastrze produkcyjnym topiki źródeł zakłada ich właściciel.
pub async fn ensure_topic(brokers: &str, topic: &str) -> anyhow::Result<()> {
    let admin: AdminClient<DefaultClientContext> = ClientConfig::new().set("bootstrap.servers", brokers).create()?;
    let new = NewTopic::new(topic, contracts::topics::PARTITIONS, TopicReplication::Fixed(1));
    let mut existed = false;
    for result in admin.create_topics(&[new], &AdminOptions::new()).await? {
        match result {
            Ok(_) => {}
            Err((_, RDKafkaErrorCode::TopicAlreadyExists)) => existed = true,
            Err((t, code)) => anyhow::bail!("creating topic {t}: {code}"),
        }
    }
    if existed {
        let name = topic.to_string();
        let count = tokio::task::spawn_blocking(move || {
            admin.inner().fetch_metadata(Some(&name), Duration::from_secs(10)).ok().and_then(|m| m.topics().first().map(|t| t.partitions().len()))
        })
        .await?;
        warn_if_few_partitions(topic, count);
    }
    Ok(())
}

/// Ostrzega, gdy istniejący topik źródłowy ma mniej partycji niż `PARTITIONS`: skalowanie instancji ingest
/// nie pomoże (partycja = jednostka podziału grupy). Partycji NIE zwiększamy automatycznie – zmieniłoby to
/// mapowanie klucz -> partycja (i kolejność per klucz).
fn warn_if_few_partitions(topic: &str, count: Option<usize>) {
    if let Some(n) = count.filter(|&n| n < contracts::topics::PARTITIONS as usize) {
        tracing::warn!(
            topic = %topic, partitions = n, expected = contracts::topics::PARTITIONS,
            "source topic has fewer partitions than expected, consumption cannot scale across ingest instances; \
             increase manually (changes key->partition mapping): rpk topic add-partitions {topic} -n {}",
            contracts::topics::PARTITIONS as usize - n
        );
    }
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

/// Przetwarza jedną wiadomość źródła. `Err` (tylko błąd publikacji) = ponowienie tej samej wiadomości
/// przez konsumenta współbieżnego (bez przeskakiwania); błąd parsowania to nie powód do ponowień:
/// zapis w `ingest_errors` + `Ok`, więc offset jest commitowany i partycja się nie blokuje.
async fn handle(ctx: &Ctx, def: &SourceDef, bytes: &[u8]) -> anyhow::Result<()> {
    let source = format!("kafka:{}", def.topic);
    match build(def, bytes) {
        Ok((format, schema, payload)) => {
            accept(&ctx.producer, &format, &schema, payload, source).await.inspect_err(|e| tracing::warn!(error = %e, "publishing to events.raw failed, retrying"))?;
        }
        Err(ParseError::Unsupported(f)) => record_error(&ctx.pool, &ctx.producer, &source, &format!("unsupported format: {f}"), bytes).await,
        Err(ParseError::Invalid(d)) => record_error(&ctx.pool, &ctx.producer, &source, &d, bytes).await,
    }
    Ok(())
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
