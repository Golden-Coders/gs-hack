//! Dedup i cykl życia alertów (tasks 4.1-4.3): wymaga Postgresa i Kafki.

mod common;

use std::time::Duration;

use ::common::db::sqlx::{self, PgPool, Row};
use contracts::{AlertChanged, AlertCommandKind, AlertState, Transition, UiEvent, UiEventKind};
use lifecycle::run_dedup_with;
use serde_json::{json, Value};
use tokio::task::JoinHandle;
use uuid::Uuid;

/// Wszystkie testy: jedna grupa + wzajemne wykluczanie (każda grupa dostaje wszystkie zdarzenia,
/// więc równoległe grupy liczyłyby je wielokrotnie). Offsety grupy przeżywają kolejne uruchomienia.
const GROUP: &str = "lifecycle-dedup-test";
const TESTS: usize = 5;
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

type Handle = (tokio::sync::watch::Sender<bool>, JoinHandle<anyhow::Result<()>>);

fn spawn_dedup(group: &str, pool: &PgPool, producer: &::common::kafka::rdkafka::producer::FutureProducer) -> Handle {
    let (tx, rx) = tokio::sync::watch::channel(false);
    let h = tokio::spawn(run_dedup_with(
        common::group(group),
        common::topics(),
        common::brokers(),
        pool.clone(),
        producer.clone(),
        rx,
    ));
    (tx, h)
}

async fn stop(h: Handle) {
    h.0.send(true).unwrap();
    h.1.await.unwrap().unwrap();
}

struct Row_ {
    id: Uuid,
    state: String,
    episode: i32,
    count: i64,
    acked_by: Option<Value>,
    resolved_by: Option<Value>,
}

async fn alert(pool: &PgPool, fp: &str) -> Option<Row_> {
    sqlx::query("SELECT id, state, episode, count, acked_by, resolved_by FROM alerts WHERE fingerprint = $1")
        .bind(fp)
        .fetch_optional(pool)
        .await
        .unwrap()
        .map(|r| Row_ {
            id: r.get("id"),
            state: r.get("state"),
            episode: r.get("episode"),
            count: r.get("count"),
            acked_by: r.get("acked_by"),
            resolved_by: r.get("resolved_by"),
        })
}

async fn wait_alert(pool: &PgPool, fp: &str, what: &str, ok: impl Fn(&Row_) -> bool) -> Row_ {
    common::wait_for(what, Duration::from_secs(40), || async { alert(pool, fp).await.filter(|r| ok(r)) }).await
}

async fn changed(fp: &str, window: Duration, n: usize) -> Vec<AlertChanged> {
    common::read_topic(&common::topics().alerts_changed, window, |c: &AlertChanged| c.fingerprint == fp, |o: &[AlertChanged]| o.len() >= n && n > 0).await
}

#[tokio::test]
async fn five_hundred_identical_events_make_one_alert() {
    let _g = SERIAL.lock().await;
    let (pool, producer) = common::setup().await;
    let fp = format!("t-500-{}", Uuid::new_v4());
    let mut cl = common::Cleanup::new(TESTS);
    cl.fingerprint(&fp);
    let d = spawn_dedup(GROUP, &pool, &producer);

    common::send_events(&producer, &fp, 500).await;
    let a = wait_alert(&pool, &fp, "count = 500", |r| r.count == 500).await;
    assert_eq!(a.state, "open");
    assert_eq!(a.episode, 1);
    let n: i64 = sqlx::query("SELECT count(*) AS n FROM alerts WHERE fingerprint = $1").bind(&fp).fetch_one(&pool).await.unwrap().get("n");
    assert_eq!(n, 1);

    // dokładnie jedno alerts.changed {opened}, brak innych (zmiany licznika idą na ui.events)
    let ch = changed(&fp, Duration::from_secs(5), 0).await;
    assert_eq!(ch.len(), 1, "{ch:?}");
    assert_eq!(ch[0].transition, Transition::Opened);
    assert_eq!(ch[0].alert_id, a.id);

    // ui.events dostaje alert_count z końcowym licznikiem
    let ui: Vec<UiEvent> = common::expect_topic(&common::topics().ui_events, 1, Duration::from_secs(20), |e: &UiEvent| {
        e.alert_id == Some(a.id) && e.kind == UiEventKind::AlertCount && e.payload["count"] == json!(500)
    })
    .await;
    assert!(ui[0].payload.get("last_seen").is_some());
    stop(d).await;
}

#[tokio::test]
async fn ack_resolve_reopen_lifecycle() {
    let _g = SERIAL.lock().await;
    let (pool, producer) = common::setup().await;
    let fp = format!("t-life-{}", Uuid::new_v4());
    let mut cl = common::Cleanup::new(TESTS);
    cl.fingerprint(&fp);
    let d = spawn_dedup(GROUP, &pool, &producer);

    common::send_events(&producer, &fp, 2).await;
    let a = wait_alert(&pool, &fp, "open, count 2", |r| r.count == 2).await;
    assert_eq!(a.state, "open");

    // ack
    common::send_command(&producer, a.id, &fp, AlertCommandKind::Ack, "anna").await;
    let acked = wait_alert(&pool, &fp, "acked", |r| r.state == "acked").await;
    assert_eq!(acked.acked_by, Some(json!({"type": "user", "person_id": "anna"})));

    // zdarzenie dla acked: licznik rośnie, stan zostaje, brak nowego alerts.changed
    common::send_events(&producer, &fp, 1).await;
    let acked = wait_alert(&pool, &fp, "count 3 in acked", |r| r.count == 3).await;
    assert_eq!(acked.state, "acked");

    // resolve komendą
    common::send_command(&producer, a.id, &fp, AlertCommandKind::Resolve, "bob").await;
    let res = wait_alert(&pool, &fp, "resolved", |r| r.state == "resolved").await;
    assert_eq!(res.resolved_by, Some(json!({"type": "user", "person_id": "bob"})));

    // reopen tego samego alertu (nowy epizod); received_at zdarzenia wyraźnie w przeszłości,
    // żeby odróżnić go od now() (Postgres trzyma µs, więc obcinamy do µs)
    let received_at = chrono::DateTime::from_timestamp_micros((chrono::Utc::now() - chrono::Duration::hours(1)).timestamp_micros()).unwrap();
    let mut ev = common::event(&fp, false);
    ev.received_at = received_at;
    ::common::kafka::publish(&producer, &common::topics().events_normalized, &fp, &ev).await.unwrap();
    let re = wait_alert(&pool, &fp, "reopened", |r| r.state == "reopened").await;
    assert_eq!(re.id, a.id);
    assert_eq!(re.episode, 2);
    assert_eq!(re.count, 1);
    assert!(re.acked_by.is_none() && re.resolved_by.is_none());

    // first_seen = received_at zdarzenia otwierającego epizod 2, nie now(); first_seen <= last_seen
    let times = sqlx::query("SELECT first_seen, last_seen FROM alerts WHERE id = $1").bind(a.id).fetch_one(&pool).await.unwrap();
    let first_seen: chrono::DateTime<chrono::Utc> = times.get("first_seen");
    let last_seen: chrono::DateTime<chrono::Utc> = times.get("last_seen");
    assert!(first_seen <= last_seen, "first_seen {first_seen} > last_seen {last_seen}");
    assert_eq!(first_seen, received_at);
    assert_eq!(last_seen, received_at);
    let ep2_opened: chrono::DateTime<chrono::Utc> = sqlx::query("SELECT opened_at FROM alert_episodes WHERE alert_id = $1 AND episode = 2")
        .bind(a.id)
        .fetch_one(&pool)
        .await
        .unwrap()
        .get("opened_at");
    assert_eq!(ep2_opened, received_at);

    let eps = sqlx::query("SELECT episode, count, acked_by, resolved_by FROM alert_episodes WHERE alert_id = $1 ORDER BY episode")
        .bind(a.id)
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(eps.len(), 2);
    assert_eq!(eps[0].get::<i64, _>("count"), 3);
    assert_eq!(eps[1].get::<i64, _>("count"), 1);
    assert!(eps[0].get::<Option<Value>, _>("acked_by").is_some());
    assert!(eps[0].get::<Option<Value>, _>("resolved_by").is_some());

    // alerts.changed: opened, acked, resolved, reopened – dokładnie te cztery, w tej kolejności
    let ch = changed(&fp, Duration::from_secs(20), 4).await;
    let ts: Vec<_> = ch.iter().map(|c| c.transition).collect();
    assert_eq!(ts, vec![Transition::Opened, Transition::Acked, Transition::Resolved, Transition::Reopened], "{ch:?}");
    assert_eq!(ch[1].state, AlertState::Acked);
    assert_eq!(ch[3].episode, 2);
    assert!(ch.iter().all(|c| c.alert_id == a.id));
    stop(d).await;
}

#[tokio::test]
async fn resolved_event_from_source_resolves_alert() {
    let _g = SERIAL.lock().await;
    let (pool, producer) = common::setup().await;
    let fp = format!("t-src-{}", Uuid::new_v4());
    let mut cl = common::Cleanup::new(TESTS);
    cl.fingerprint(&fp);
    let d = spawn_dedup(GROUP, &pool, &producer);

    common::send_events(&producer, &fp, 1).await;
    wait_alert(&pool, &fp, "open", |r| r.state == "open").await;
    let ev = common::event(&fp, true);
    ::common::kafka::publish(&producer, &common::topics().events_normalized, &fp, &ev).await.unwrap();
    let r = wait_alert(&pool, &fp, "resolved", |r| r.state == "resolved").await;
    assert_eq!(r.resolved_by, Some(json!({"type": "source"})));

    let ch = changed(&fp, Duration::from_secs(15), 2).await;
    assert_eq!(ch.len(), 2, "{ch:?}");
    assert_eq!(ch[1].transition, Transition::Resolved);
    assert_eq!(ch[1].actor, Some(contracts::Actor::Source));
    stop(d).await;
}

#[tokio::test]
async fn resolved_event_for_unknown_fingerprint_creates_nothing() {
    let _g = SERIAL.lock().await;
    let (pool, producer) = common::setup().await;
    let fp = format!("t-nores-{}", Uuid::new_v4());
    let mut cl = common::Cleanup::new(TESTS);
    cl.fingerprint(&fp);
    let d = spawn_dedup(GROUP, &pool, &producer);
    ::common::kafka::publish(&producer, &common::topics().events_normalized, &fp, &common::event(&fp, true)).await.unwrap();
    // marker: kolejne zdarzenie innego fingerprintu przejdzie po poprzednim tylko w tej samej partycji, więc po prostu czekamy
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert!(alert(&pool, &fp).await.is_none());
    stop(d).await;
}

#[tokio::test]
async fn two_instances_same_group_keep_single_alert() {
    let _g = SERIAL.lock().await;
    let (pool, producer) = common::setup().await;
    let fp = format!("t-two-{}", Uuid::new_v4());
    let mut cl = common::Cleanup::new(TESTS);
    cl.fingerprint(&fp);
    let group = GROUP;
    let a = spawn_dedup(group, &pool, &producer);
    let b = spawn_dedup(group, &pool, &producer);

    common::send_events(&producer, &fp, 100).await;
    wait_alert(&pool, &fp, "count 100", |r| r.count == 100).await;

    // kolejne 100 w trakcie zatrzymywania jednej instancji
    let p2 = producer.clone();
    let fp2 = fp.clone();
    let sender = tokio::spawn(async move {
        for _ in 0..10 {
            common::send_events(&p2, &fp2, 10).await;
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    });
    tokio::time::sleep(Duration::from_millis(800)).await;
    stop(a).await;
    sender.await.unwrap();

    // czekamy, aż licznik się ustabilizuje
    let mut last = (0, std::time::Instant::now());
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(90) {
        let c = alert(&pool, &fp).await.unwrap().count;
        if c != last.0 {
            last = (c, std::time::Instant::now());
        } else if last.1.elapsed() > Duration::from_secs(8) && c > 100 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let n: i64 = sqlx::query("SELECT count(*) AS n FROM alerts WHERE fingerprint = $1").bind(&fp).fetch_one(&pool).await.unwrap().get("n");
    assert_eq!(n, 1);
    let c = alert(&pool, &fp).await.unwrap().count;
    println!("final count = {c}");
    assert!(c > 100 && c <= 200, "count = {c}");
    let ch = changed(&fp, Duration::from_secs(5), 0).await;
    assert_eq!(ch.iter().filter(|c| c.transition == Transition::Opened).count(), 1);
    stop(b).await;
}
