//! Wspólne helpery testów integracyjnych (wymagają Postgresa i Kafki z docker compose).
#![allow(dead_code)]

use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use ::common::kafka::rdkafka::admin::{AdminClient, AdminOptions, NewTopic, TopicReplication};
use ::common::kafka::rdkafka::client::DefaultClientContext;
use ::common::kafka::rdkafka::config::ClientConfig;
use ::common::kafka::rdkafka::types::RDKafkaErrorCode;

use common::db::sqlx::PgPool;
use common::kafka::rdkafka::consumer::StreamConsumer;
use common::kafka::rdkafka::producer::FutureProducer;
use contracts::{Actor, AlertCommand, AlertCommandKind, CanonicalEvent, Envelope, Severity};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use uuid::Uuid;

pub fn brokers() -> String {
    std::env::var("KAFKA_BROKERS").unwrap_or_else(|_| "localhost:19092".into())
}

pub fn database_url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://alerting:alerting@localhost:5432/alerting".into())
}

/// Prefiks topików tego procesu testowego (każdy plik testów to osobny proces) – izoluje testy
/// od instancji `lifecycle` działającej w compose, która czyta topiki produkcyjne.
pub fn prefix() -> &'static str {
    static P: OnceLock<String> = OnceLock::new();
    P.get_or_init(|| format!("test-{}", Uuid::new_v4().simple()))
}

pub fn topics() -> lifecycle::Topics {
    lifecycle::Topics::with_prefix(prefix())
}

/// Nazwa grupy konsumentów unikalna dla procesu (nigdy produkcyjna).
pub fn group(name: &str) -> &'static str {
    Box::leak(format!("{name}-{}", prefix()).into_boxed_str())
}

fn topic_names() -> Vec<String> {
    let t = topics();
    vec![t.events_raw, t.events_unmapped, t.events_normalized, t.alerts_commands, t.alerts_changed, t.ui_events]
}

fn admin() -> AdminClient<DefaultClientContext> {
    ClientConfig::new().set("bootstrap.servers", brokers()).create().expect("kafka admin")
}

/// Mniej niż produkcyjne 12: broker w compose ma twardy limit 256 partycji (a testy ingest zostawiają
/// topiki-śmieci). Dwie partycje wystarczą dla topików konsumowanych przez dedup (test dwóch instancji).
fn partitions(topic: &str) -> i32 {
    if topic.ends_with("events.normalized") || topic.ends_with("alerts.commands") {
        2
    } else {
        1
    }
}

/// Zakłada 6 topików prefiksu (Redpanda nie tworzy ich automatycznie); `TopicAlreadyExists` = OK.
async fn ensure_topics() {
    let names = topic_names();
    let new: Vec<_> = names.iter().map(|n| NewTopic::new(n, partitions(n), TopicReplication::Fixed(1))).collect();
    let admin = admin();
    for r in admin.create_topics(new.iter(), &AdminOptions::new()).await.expect("create topics") {
        match r {
            Ok(_) | Err((_, RDKafkaErrorCode::TopicAlreadyExists)) => {}
            Err((t, code)) => panic!("creating topic {t}: {code}"),
        }
    }
}

async fn delete_topics() {
    let names = topic_names();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let _ = admin().delete_topics(&refs, &AdminOptions::new()).await;
}

/// Sprzątanie po teście (także przy panice): usuwa z bazy wiersze testu, a po ostatnim teście
/// pliku – topiki prefiksu. Działa w osobnym wątku z własnym runtime (Drop nie jest async).
pub struct Cleanup {
    total: usize,
    fingerprints: Vec<String>,
    schemas: Vec<String>,
}

static FINISHED: AtomicUsize = AtomicUsize::new(0);

impl Cleanup {
    /// `total` = liczba testów w pliku (po ostatnim usuwamy topiki).
    pub fn new(total: usize) -> Self {
        Self { total, fingerprints: vec![], schemas: vec![] }
    }
    pub fn fingerprint(&mut self, fp: &str) {
        self.fingerprints.push(fp.to_string());
    }
    pub fn schema(&mut self, schema: &str) {
        self.schemas.push(schema.to_string());
    }
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let last = FINISHED.fetch_add(1, Ordering::SeqCst) + 1 == self.total;
        let (fps, schemas) = (std::mem::take(&mut self.fingerprints), std::mem::take(&mut self.schemas));
        let _ = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
                if let Ok(pool) = PgPool::connect(&database_url()).await {
                    use ::common::db::sqlx;
                    for q in [
                        "DELETE FROM alert_episodes WHERE alert_id IN (SELECT id FROM alerts WHERE fingerprint = ANY($1))",
                        "DELETE FROM alerts WHERE fingerprint = ANY($1)",
                    ] {
                        let _ = sqlx::query(q).bind(&fps).execute(&pool).await;
                    }
                    for q in ["DELETE FROM unmapped_events WHERE schema = ANY($1)", "DELETE FROM mappings WHERE schema = ANY($1)"] {
                        let _ = sqlx::query(q).bind(&schemas).execute(&pool).await;
                    }
                }
                if last {
                    delete_topics().await;
                }
            })
        })
        .join();
    }
}

pub async fn setup() -> (PgPool, FutureProducer) {
    static TOPICS: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();
    TOPICS.get_or_init(ensure_topics).await;
    let pool = common::db::connect_and_migrate(&database_url()).await.expect("postgres");
    let producer = common::kafka::producer(&brokers()).expect("kafka producer");
    (pool, producer)
}

/// Odpytuje `check` co 200 ms aż zwróci `Some`, albo panikuje po `timeout`.
pub async fn wait_for<T, F, Fut>(what: &str, timeout: Duration, mut check: F) -> T
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Option<T>>,
{
    let start = Instant::now();
    loop {
        if let Some(v) = check().await {
            return v;
        }
        assert!(start.elapsed() < timeout, "timeout czekając na: {what}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Czyta topik od początku (świeża, losowa grupa) przez `window` i zwraca zdekodowane wiadomości
/// spełniające `pred`. Kończy wcześniej tylko przez `until` (zwraca true, gdy zebrano dość).
pub async fn read_topic<T: DeserializeOwned>(
    topic: &str,
    window: Duration,
    pred: impl Fn(&T) -> bool,
    until: impl Fn(&[T]) -> bool,
) -> Vec<T> {
    let consumer: StreamConsumer = common::kafka::consumer(&brokers(), &format!("test-reader-{}", Uuid::new_v4()), &[topic]).unwrap();
    let mut out = Vec::new();
    let start = Instant::now();
    while start.elapsed() < window {
        let left = window - start.elapsed();
        if let Ok(Ok(m)) = tokio::time::timeout(left.min(Duration::from_millis(500)), consumer.recv()).await {
            if let Ok(v) = common::kafka::decode::<T>(&m) {
                if pred(&v) {
                    out.push(v);
                }
            }
        }
        if until(&out) {
            break;
        }
    }
    out
}

/// Czeka, aż w topiku pojawi się >= `n` wiadomości spełniających `pred` (maks. `timeout`).
pub async fn expect_topic<T: DeserializeOwned>(topic: &str, n: usize, timeout: Duration, pred: impl Fn(&T) -> bool) -> Vec<T> {
    let v = read_topic(topic, timeout, pred, |o: &[T]| o.len() >= n).await;
    assert!(v.len() >= n, "w topiku {topic} oczekiwano >= {n} wiadomości, jest {}", v.len());
    v
}

pub fn envelope(schema: &str, payload: Value) -> Envelope {
    Envelope {
        event_id: Uuid::new_v4(),
        format: "json".into(),
        schema: schema.into(),
        payload,
        source: "test".into(),
        received_at: chrono::Utc::now(),
    }
}

pub fn event(fingerprint: &str, resolved: bool) -> CanonicalEvent {
    CanonicalEvent {
        event_id: Uuid::new_v4(),
        schema: "test.schema".into(),
        source: "test".into(),
        severity: Severity::Error,
        title: format!("title {fingerprint}"),
        fingerprint: fingerprint.into(),
        resolved,
        fields: serde_json::Map::new(),
        raw: json!({"x": 1}),
        received_at: chrono::Utc::now(),
        mapping_version: 1,
    }
}

pub async fn send_events(producer: &FutureProducer, fingerprint: &str, n: usize) {
    for chunk in (0..n).collect::<Vec<_>>().chunks(100) {
        let futs = chunk.iter().map(|_| {
            let ev = event(fingerprint, false);
            async move { common::kafka::publish(producer, &topics().events_normalized, &ev.fingerprint, &ev).await.unwrap() }
        });
        futures::future::join_all(futs).await;
    }
}

pub async fn send_command(producer: &FutureProducer, alert_id: Uuid, fingerprint: &str, kind: AlertCommandKind, person: &str) {
    let cmd = AlertCommand {
        alert_id,
        fingerprint: fingerprint.into(),
        kind,
        actor: Actor::User { person_id: person.into() },
        at: chrono::Utc::now(),
    };
    common::kafka::publish(producer, &topics().alerts_commands, fingerprint, &cmd).await.unwrap();
}
