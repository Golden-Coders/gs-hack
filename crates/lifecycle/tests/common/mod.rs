//! Wspólne helpery testów integracyjnych (wymagają Postgresa i Kafki z docker compose).
#![allow(dead_code)]

use std::future::Future;
use std::time::{Duration, Instant};

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

pub async fn setup() -> (PgPool, FutureProducer) {
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
            async move { common::kafka::publish(producer, contracts::topics::EVENTS_NORMALIZED, &ev.fingerprint, &ev).await.unwrap() }
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
    common::kafka::publish(producer, contracts::topics::ALERTS_COMMANDS, fingerprint, &cmd).await.unwrap();
}
