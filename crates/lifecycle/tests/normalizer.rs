//! Normalizer (tasks 3.2, 3.3, 3.5): wymaga Postgresa i Kafki.

mod common;

use std::time::Duration;

use ::common::db::sqlx::{self, Row};
use contracts::{CanonicalEvent, Envelope, Severity};
use lifecycle::config::{load_mappings, new_store};
use lifecycle::{replay_unmapped_to, run_normalizer_with};
use serde_json::json;
use uuid::Uuid;

/// Jedna grupa i wzajemne wykluczanie: normalizer z cudzym `store` zrobiłby z naszych zdarzeń UNMAPPED.
const GROUP: &str = "lifecycle-normalizer-test";
const TESTS: usize = 2;
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn mapping_def(schema: &str) -> serde_json::Value {
    json!({
        "schema": schema,
        "fields": {"host": "$.host"},
        "title": "$.msg",
        "severity": {"type": "const", "value": "error"},
        "fingerprint": ["$.id"]
    })
}

async fn insert_mapping(pool: &sqlx::PgPool, schema: &str) {
    sqlx::query("INSERT INTO mappings (schema, def, version) VALUES ($1, $2, 1)")
        .bind(schema)
        .bind(mapping_def(schema))
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn mapped_event_goes_to_normalized_keyed_by_fingerprint() {
    let _g = SERIAL.lock().await;
    let (pool, producer) = common::setup().await;
    let schema = format!("t-{}", Uuid::new_v4());
    let mut cl = common::Cleanup::new(TESTS);
    cl.schema(&schema);
    insert_mapping(&pool, &schema).await;
    let store = new_store();
    load_mappings(&pool, &store).await.unwrap();
    let (tx, rx) = tokio::sync::watch::channel(false);
    let h = tokio::spawn(run_normalizer_with(common::group(GROUP), common::topics(), common::brokers(), pool.clone(), producer.clone(), store, rx));

    let env = common::envelope(&schema, json!({"id": "A1", "msg": "hello", "host": "h1"}));
    ::common::kafka::publish(&producer, &common::topics().events_raw, &env.event_id.to_string(), &env).await.unwrap();

    let want = format!("{schema}|A1");
    let evs: Vec<CanonicalEvent> = common::expect_topic(&common::topics().events_normalized, 1, Duration::from_secs(30), |e: &CanonicalEvent| e.fingerprint == want).await;
    let ev = &evs[0];
    assert_eq!(ev.event_id, env.event_id);
    assert_eq!(ev.title, "hello");
    assert_eq!(ev.severity, Severity::Error);
    assert_eq!(ev.fields["host"], "h1");
    assert_eq!(ev.raw, env.payload);
    assert_eq!(ev.mapping_version, 1);

    // klucz wiadomości = fingerprint
    use ::common::kafka::rdkafka::message::Message;
    let c = ::common::kafka::consumer(&common::brokers(), &format!("test-reader-{}", Uuid::new_v4()), &[common::topics().events_normalized.as_str()]).unwrap();
    let start = std::time::Instant::now();
    let mut key = None;
    while key.is_none() && start.elapsed() < Duration::from_secs(30) {
        if let Ok(Ok(m)) = tokio::time::timeout(Duration::from_millis(500), c.recv()).await {
            if let Ok(e) = ::common::kafka::decode::<CanonicalEvent>(&m) {
                if e.fingerprint == want {
                    key = m.key().map(|k| String::from_utf8_lossy(k).to_string());
                }
            }
        }
    }
    assert_eq!(key.as_deref(), Some(want.as_str()));

    tx.send(true).unwrap();
    h.await.unwrap().unwrap();
}

#[tokio::test]
async fn unknown_schema_is_unmapped_then_replayed_after_mapping_created() {
    let _g = SERIAL.lock().await;
    let (pool, producer) = common::setup().await;
    let schema = format!("t-{}", Uuid::new_v4());
    let mut cl = common::Cleanup::new(TESTS);
    cl.schema(&schema);
    let store = new_store();
    load_mappings(&pool, &store).await.unwrap();
    let (tx, rx) = tokio::sync::watch::channel(false);
    let h = tokio::spawn(run_normalizer_with(common::group(GROUP), common::topics(), common::brokers(), pool.clone(), producer.clone(), store.clone(), rx));

    for i in 0..3 {
        let env = common::envelope(&schema, json!({"id": format!("U{i}"), "msg": "m", "host": "h"}));
        ::common::kafka::publish(&producer, &common::topics().events_raw, &env.event_id.to_string(), &env).await.unwrap();
    }

    // wiersze w unmapped_events + wiadomości na events.unmapped
    common::wait_for("3 wiersze unmapped_events", Duration::from_secs(30), || async {
        let n: i64 = sqlx::query("SELECT count(*) AS n FROM unmapped_events WHERE schema = $1")
            .bind(&schema)
            .fetch_one(&pool)
            .await
            .unwrap()
            .get("n");
        (n == 3).then_some(())
    })
    .await;
    let un: Vec<Envelope> = common::expect_topic(&common::topics().events_unmapped, 3, Duration::from_secs(30), |e: &Envelope| e.schema == schema).await;
    assert_eq!(un.len(), 3);
    // nic nie trafiło jeszcze na events.normalized
    let early: Vec<CanonicalEvent> = common::read_topic(&common::topics().events_normalized, Duration::from_secs(3), |e: &CanonicalEvent| e.fingerprint.starts_with(&schema), |_| false).await;
    assert!(early.is_empty());

    // "Utworzenie mappingu z UI": INSERT + przeładowanie + replay
    insert_mapping(&pool, &schema).await;
    load_mappings(&pool, &store).await.unwrap();
    replay_unmapped_to(&pool, &producer, &store, &common::topics().events_raw).await.unwrap();

    let evs: Vec<CanonicalEvent> = common::expect_topic(&common::topics().events_normalized, 3, Duration::from_secs(30), |e: &CanonicalEvent| e.fingerprint.starts_with(&schema)).await;
    let mut fps: Vec<_> = evs.iter().map(|e| e.fingerprint.clone()).collect();
    fps.sort();
    assert_eq!(fps, vec![format!("{schema}|U0"), format!("{schema}|U1"), format!("{schema}|U2")]);
    let left: i64 = sqlx::query("SELECT count(*) AS n FROM unmapped_events WHERE schema = $1")
        .bind(&schema)
        .fetch_one(&pool)
        .await
        .unwrap()
        .get("n");
    assert_eq!(left, 0);

    tx.send(true).unwrap();
    h.await.unwrap().unwrap();
}
