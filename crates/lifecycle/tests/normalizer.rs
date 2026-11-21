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
const TESTS: usize = 4;
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
    let h = tokio::spawn(run_normalizer_with(
        common::group(GROUP),
        common::topics(),
        common::brokers(),
        pool.clone(),
        producer.clone(),
        store,
        rx,
    ));

    let env = common::envelope(&schema, json!({"id": "A1", "msg": "hello", "host": "h1"}));
    ::common::kafka::publish(
        &producer,
        &common::topics().events_raw,
        &env.event_id.to_string(),
        &env,
    )
    .await
    .unwrap();

    let want = format!("{schema}|A1");
    let evs: Vec<CanonicalEvent> = common::expect_topic(
        &common::topics().events_normalized,
        1,
        Duration::from_secs(30),
        |e: &CanonicalEvent| e.fingerprint == want,
    )
    .await;
    let ev = &evs[0];
    assert_eq!(ev.event_id, env.event_id);
    assert_eq!(ev.title, "hello");
    assert_eq!(ev.severity, Severity::Error);
    assert_eq!(ev.fields["host"], "h1");
    assert_eq!(ev.raw, env.payload);
    assert_eq!(ev.mapping_version, 1);

    use ::common::kafka::rdkafka::message::Message;
    let c = ::common::kafka::consumer(
        &common::brokers(),
        &format!("test-reader-{}", Uuid::new_v4()),
        &[common::topics().events_normalized.as_str()],
    )
    .unwrap();
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
    let h = tokio::spawn(run_normalizer_with(
        common::group(GROUP),
        common::topics(),
        common::brokers(),
        pool.clone(),
        producer.clone(),
        store.clone(),
        rx,
    ));

    for i in 0..3 {
        let env = common::envelope(
            &schema,
            json!({"id": format!("U{i}"), "msg": "m", "host": "h"}),
        );
        ::common::kafka::publish(
            &producer,
            &common::topics().events_raw,
            &env.event_id.to_string(),
            &env,
        )
        .await
        .unwrap();
    }

    common::wait_for(
        "3 wiersze unmapped_events",
        Duration::from_secs(30),
        || async {
            let n: i64 = sqlx::query("SELECT count(*) AS n FROM unmapped_events WHERE schema = $1")
                .bind(&schema)
                .fetch_one(&pool)
                .await
                .unwrap()
                .get("n");
            (n == 3).then_some(())
        },
    )
    .await;
    let un: Vec<Envelope> = common::expect_topic(
        &common::topics().events_unmapped,
        3,
        Duration::from_secs(30),
        |e: &Envelope| e.schema == schema,
    )
    .await;
    assert_eq!(un.len(), 3);
    let early: Vec<CanonicalEvent> = common::read_topic(
        &common::topics().events_normalized,
        Duration::from_secs(3),
        |e: &CanonicalEvent| e.fingerprint.starts_with(&schema),
        |_| false,
    )
    .await;
    assert!(early.is_empty());

    insert_mapping(&pool, &schema).await;
    load_mappings(&pool, &store).await.unwrap();
    replay_unmapped_to(&pool, &producer, &store, &common::topics().events_raw)
        .await
        .unwrap();

    let evs: Vec<CanonicalEvent> = common::expect_topic(
        &common::topics().events_normalized,
        3,
        Duration::from_secs(30),
        |e: &CanonicalEvent| e.fingerprint.starts_with(&schema),
    )
    .await;
    let mut fps: Vec<_> = evs.iter().map(|e| e.fingerprint.clone()).collect();
    fps.sort();
    assert_eq!(
        fps,
        vec![
            format!("{schema}|U0"),
            format!("{schema}|U1"),
            format!("{schema}|U2")
        ]
    );
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

/// Uruchamia normalizer na własnej grupie i zwraca (handle, sygnał stopu).
fn spawn_normalizer(
    group: &'static str,
    pool: &sqlx::PgPool,
    producer: &::common::kafka::rdkafka::producer::FutureProducer,
    store: lifecycle::config::MappingStore,
) -> (
    tokio::task::JoinHandle<anyhow::Result<()>>,
    tokio::sync::watch::Sender<bool>,
) {
    let (tx, rx) = tokio::sync::watch::channel(false);
    let h = tokio::spawn(run_normalizer_with(
        group,
        common::topics(),
        common::brokers(),
        pool.clone(),
        producer.clone(),
        store,
        rx,
    ));
    (h, tx)
}

/// Różnica między high watermarkiem `events.raw` (partycja 0) a committed offsetem grupy.
fn raw_lag(group: &str) -> i64 {
    use ::common::kafka::rdkafka::config::ClientConfig;
    use ::common::kafka::rdkafka::consumer::{Consumer, StreamConsumer};
    use ::common::kafka::rdkafka::{Offset, TopicPartitionList};
    let topic = common::topics().events_raw;
    let c: StreamConsumer = ClientConfig::new()
        .set("bootstrap.servers", common::brokers())
        .set("group.id", group)
        .set("enable.auto.commit", "false")
        .create()
        .unwrap();
    let (_, high) = c
        .fetch_watermarks(&topic, 0, Duration::from_secs(10))
        .unwrap();
    let mut tpl = TopicPartitionList::new();
    tpl.add_partition_offset(&topic, 0, Offset::Invalid)
        .unwrap();
    let committed = match c
        .committed_offsets(tpl, Duration::from_secs(10))
        .unwrap()
        .elements()[0]
        .offset()
    {
        Offset::Offset(o) => o,
        _ => 0,
    };
    high - committed
}

/// Przepustowość: 2000 zdarzeń, wszystkie z poprawnym fingerprintem na `events.normalized`,
/// a po shutdownie offset grupy = koniec topiku (commit po skutkach, nic nie zgubione ani niezacommitowane).
#[tokio::test(flavor = "multi_thread")]
async fn concurrent_throughput_all_normalized_and_offsets_committed() {
    let _g = SERIAL.lock().await;
    const N: usize = 2000;
    let (pool, producer) = common::setup().await;
    let schema = format!("t-{}", Uuid::new_v4());
    let mut cl = common::Cleanup::new(TESTS);
    cl.schema(&schema);
    insert_mapping(&pool, &schema).await;
    let store = new_store();
    load_mappings(&pool, &store).await.unwrap();

    let raw_topic = common::topics().events_raw;
    for chunk in (0..N).collect::<Vec<_>>().chunks(200) {
        let futs = chunk.iter().map(|i| {
            let env = common::envelope(
                &schema,
                json!({"id": format!("P{i}"), "msg": "m", "host": "h"}),
            );
            let producer = producer.clone();
            let topic = raw_topic.clone();
            async move {
                ::common::kafka::publish(&producer, &topic, &env.event_id.to_string(), &env)
                    .await
                    .unwrap()
            }
        });
        futures::future::join_all(futs).await;
    }

    let group_name = common::group("lifecycle-normalizer-thr");
    let started = std::time::Instant::now();
    let (h, tx) = spawn_normalizer(group_name, &pool, &producer, store);
    let evs: Vec<CanonicalEvent> = common::expect_topic(
        &common::topics().events_normalized,
        N,
        Duration::from_secs(120),
        |e: &CanonicalEvent| e.fingerprint.starts_with(&format!("{schema}|P")),
    )
    .await;
    println!(
        "normalizer throughput: {N} zdarzeń w {:?}",
        started.elapsed()
    );
    let mut fps: Vec<_> = evs.iter().map(|e| e.fingerprint.clone()).collect();
    fps.sort();
    fps.dedup();
    assert_eq!(fps.len(), N);
    assert!((0..N).all(|i| fps.binary_search(&format!("{schema}|P{i}")).is_ok()));

    tx.send(true).unwrap();
    h.await.unwrap().unwrap();
    assert_eq!(
        raw_lag(group_name),
        0,
        "po shutdownie committed offset = koniec events.raw"
    );
}

/// Kolejność per klucz: wiadomości o tym samym kluczu `events.raw` i tym samym fingerprincie
/// wychodzą na `events.normalized` (ta sama partycja) w kolejności publikacji.
#[tokio::test(flavor = "multi_thread")]
async fn order_preserved_per_key() {
    let _g = SERIAL.lock().await;
    const KEYS: usize = 5;
    const PER_KEY: usize = 40;
    let (pool, producer) = common::setup().await;
    let schema = format!("t-{}", Uuid::new_v4());
    let mut cl = common::Cleanup::new(TESTS);
    cl.schema(&schema);
    insert_mapping(&pool, &schema).await;
    let store = new_store();
    load_mappings(&pool, &store).await.unwrap();
    let (h, tx) = spawn_normalizer(
        common::group("lifecycle-normalizer-ord"),
        &pool,
        &producer,
        store,
    );

    let raw_topic = common::topics().events_raw;
    for seq in 0..PER_KEY {
        for k in 0..KEYS {
            let env = common::envelope(
                &schema,
                json!({"id": format!("K{k}"), "msg": format!("{seq:04}"), "host": "h"}),
            );
            ::common::kafka::publish(&producer, &raw_topic, &format!("{schema}-key{k}"), &env)
                .await
                .unwrap();
        }
    }

    let evs: Vec<CanonicalEvent> = common::expect_topic(
        &common::topics().events_normalized,
        KEYS * PER_KEY,
        Duration::from_secs(60),
        |e: &CanonicalEvent| e.fingerprint.starts_with(&format!("{schema}|K")),
    )
    .await;
    for k in 0..KEYS {
        let fp = format!("{schema}|K{k}");
        let titles: Vec<&str> = evs
            .iter()
            .filter(|e| e.fingerprint == fp)
            .map(|e| e.title.as_str())
            .collect();
        assert_eq!(titles.len(), PER_KEY);
        assert!(
            titles.windows(2).all(|w| w[0] < w[1]),
            "kolejność naruszona dla {fp}: {titles:?}"
        );
    }

    tx.send(true).unwrap();
    h.await.unwrap().unwrap();
}
