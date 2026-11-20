//! Testy integracyjne: wymagają działającego Postgresa i Kafki (docker compose).

use std::{sync::Arc, time::Duration};

use common::kafka::{
    self,
    rdkafka::{
        producer::{FutureProducer, FutureRecord},
    },
};
use contracts::{topics, Envelope};
use ingest::{http, sources};
use reqwest::StatusCode;
use serde_json::{json, Value};
use uuid::Uuid;

const BROKERS: &str = "localhost:19092";
const DB: &str = "postgres://alerting:alerting@localhost:5432/alerting";

fn producer() -> FutureProducer {
    kafka::producer(BROKERS).unwrap()
}

async fn start_http() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = http::app(http::AppState { producer: producer() });
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{addr}")
}

/// Szuka na `events.raw` koperty ze wskazanym `schema` (nowa grupa, od początku).
async fn find_raw(schema: &str, timeout: Duration) -> Option<Envelope> {
    let consumer = kafka::consumer(BROKERS, &format!("test-{}", Uuid::new_v4()), &[topics::EVENTS_RAW]).unwrap();
    let deadline = tokio::time::Instant::now() + timeout;
    while let Ok(res) = tokio::time::timeout_at(deadline, consumer.recv()).await {
        let Ok(msg) = res else { continue };
        if let Ok(env) = kafka::decode::<Envelope>(&msg) {
            if env.schema == schema {
                return Some(env);
            }
        }
    }
    None
}

async fn expect_raw(schema: &str) -> Envelope {
    find_raw(schema, Duration::from_secs(15)).await.expect("no envelope on events.raw")
}

fn uniq(prefix: &str) -> String {
    format!("{prefix}.{}", Uuid::new_v4().simple())
}

#[tokio::test]
async fn post_ingest_valid_envelope() {
    let base = start_http().await;
    let schema = uniq("t.valid");
    let r = reqwest::Client::new().post(format!("{base}/ingest")).json(&json!({"format":"json","schema":schema,"payload":{"a":1}})).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::ACCEPTED);
    let body: Value = r.json().await.unwrap();
    let id: Uuid = body["event_id"].as_str().unwrap().parse().unwrap();
    let env = expect_raw(&schema).await;
    assert_eq!(env.event_id, id);
    assert_eq!(env.source, "http");
    assert_eq!(env.payload, json!({"a": 1}));
    assert!((chrono::Utc::now() - env.received_at).num_seconds() < 60);
}

#[tokio::test]
async fn post_ingest_missing_fields_400_and_nothing_published() {
    let base = start_http().await;
    let c = reqwest::Client::new();
    let schema = uniq("t.missing");
    // brak payload
    let r = c.post(format!("{base}/ingest")).json(&json!({"format":"json","schema":schema})).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    assert!(r.text().await.unwrap().contains("payload"));
    // brak schema
    let r = c.post(format!("{base}/ingest")).json(&json!({"format":"json","payload":{"marker":schema}})).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    assert!(r.text().await.unwrap().contains("schema"));
    assert!(find_raw(&schema, Duration::from_secs(5)).await.is_none());
}

#[tokio::test]
async fn post_ingest_non_json_body_400() {
    let base = start_http().await;
    let r = reqwest::Client::new().post(format!("{base}/ingest")).body("not json").send().await.unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn post_ingest_xml_415_lists_formats() {
    let base = start_http().await;
    let schema = uniq("t.xml");
    let r = reqwest::Client::new().post(format!("{base}/ingest")).json(&json!({"format":"xml","schema":schema,"payload":"<a/>"})).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    let t = r.text().await.unwrap();
    assert!(t.contains("json") && t.contains("yaml"), "{t}");
    assert!(find_raw(&schema, Duration::from_secs(3)).await.is_none());
}

#[tokio::test]
async fn post_ingest_schema_json_variants() {
    let base = start_http().await;
    let c = reqwest::Client::new();
    for ct in ["application/json", "application/json; charset=utf-8"] {
        let schema = uniq("t.rawjson");
        let r = c.post(format!("{base}/ingest/{schema}")).header("content-type", ct).body(r#"{"alert":"x"}"#).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::ACCEPTED, "{ct}");
        let env = expect_raw(&schema).await;
        assert_eq!(env.format, "json");
        assert_eq!(env.source, format!("http:{schema}"));
        assert_eq!(env.payload, json!({"alert": "x"}));
    }
}

#[tokio::test]
async fn post_ingest_schema_yaml() {
    let base = start_http().await;
    let schema = uniq("t.yaml");
    let r = reqwest::Client::new().post(format!("{base}/ingest/{schema}")).header("content-type", "application/yaml").body("a: 1\nb: 2\n").send().await.unwrap();
    assert_eq!(r.status(), StatusCode::ACCEPTED);
    let env = expect_raw(&schema).await;
    assert_eq!(env.format, "yaml");
    assert_eq!(env.payload, json!("a: 1\nb: 2\n"));
}

#[tokio::test]
async fn post_ingest_schema_bad_requests() {
    let base = start_http().await;
    let c = reqwest::Client::new();
    let r = c.post(format!("{base}/ingest/x")).header("content-type", "text/plain").body("hi").send().await.unwrap();
    assert_eq!(r.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    let r = c.post(format!("{base}/ingest/x")).header("content-type", "application/json").body("{nope").send().await.unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    let r = c.post(format!("{base}/ingest/x")).header("content-type", "application/yaml").body("a: [oops").send().await.unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn kafka_source_added_without_restart() {
    let dir = std::env::temp_dir().join(format!("ingest-src-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let pool = common::db::connect_and_migrate(DB).await.unwrap();
    let p = producer();
    let ctx = Arc::new(sources::Ctx { brokers: BROKERS.into(), pool: pool.clone(), producer: p.clone() });
    let scanner = tokio::spawn(sources::run_scanner(ctx, dir.to_string_lossy().into()));

    // źródło dodane PO starcie skanera
    tokio::time::sleep(Duration::from_millis(500)).await;
    let topic = uniq("test.src");
    let schema = uniq("t.kafka");
    std::fs::write(dir.join("s.yaml"), format!("topic: {topic}\nschema: {schema}\nformat: json\n")).unwrap();
    // poczekaj, aż skaner (2 s) uruchomi konsumenta, i wyślij wiadomości (earliest => nie zginą)
    tokio::time::sleep(Duration::from_secs(4)).await;
    let send = |payload: &'static str| {
        let p = p.clone();
        let topic = topic.clone();
        async move {
            p.send(FutureRecord::to(&topic).key("k").payload(payload), Duration::from_secs(10)).await.map_err(|(e, _)| e).unwrap();
        }
    };
    send("{not json").await;
    send(r#"{"k":"v"}"#).await;

    let env = expect_raw(&schema).await;
    assert_eq!(env.source, format!("kafka:{topic}"));
    assert_eq!(env.format, "json");
    assert_eq!(env.payload, json!({"k": "v"}));

    // błąd parsowania -> ingest_errors
    let source = format!("kafka:{topic}");
    let mut n = 0i64;
    for _ in 0..30 {
        n = common::db::sqlx::query_scalar("SELECT count(*) FROM ingest_errors WHERE source = $1").bind(&source).fetch_one(&pool).await.unwrap();
        if n > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert_eq!(n, 1);
    scanner.abort();
    let _ = std::fs::remove_dir_all(&dir);
}
