//! Współbieżny dedup (etap 4): kolejność per fingerprint, liczniki, rebalans, shutdown.
//! Wymaga Postgresa i Kafki (jak pozostałe testy integracyjne).

mod common;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use ::common::db::sqlx::{self, PgPool, Row};
use ::common::kafka::rdkafka::config::ClientConfig;
use ::common::kafka::rdkafka::consumer::{BaseConsumer, Consumer};
use ::common::kafka::rdkafka::producer::FutureProducer;
use ::common::kafka::rdkafka::{Offset, TopicPartitionList};
use contracts::{AlertChanged, AlertCommandKind, Transition};
use lifecycle::run_dedup_with;
use tokio::task::JoinHandle;
use uuid::Uuid;

/// Jedna grupa dla wszystkich testów pliku + wzajemne wykluczanie (offsety grupy przechodzą między testami,
/// więc zdarzenia jednego testu nie są czytane drugi raz).
const GROUP: &str = "lifecycle-dedup-concurrent-test";
const TESTS: usize = 4;
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

type Handle = (tokio::sync::watch::Sender<bool>, JoinHandle<anyhow::Result<()>>);

fn spawn_dedup(pool: &PgPool, producer: &FutureProducer) -> Handle {
    let (tx, rx) = tokio::sync::watch::channel(false);
    let h = tokio::spawn(run_dedup_with(common::group(GROUP), common::topics(), common::brokers(), pool.clone(), producer.clone(), rx));
    (tx, h)
}

async fn stop(h: Handle) {
    h.0.send(true).unwrap();
    h.1.await.unwrap().unwrap();
}

/// Liczba alertów i suma liczników dla fingerprintów o danym prefiksie.
async fn totals(pool: &PgPool, prefix: &str) -> (i64, i64) {
    let r = sqlx::query("SELECT count(*) AS n, COALESCE(sum(count), 0)::bigint AS s FROM alerts WHERE fingerprint LIKE $1")
        .bind(format!("{prefix}%"))
        .fetch_one(pool)
        .await
        .unwrap();
    (r.get("n"), r.get("s"))
}

async fn wait_sum(pool: &PgPool, prefix: &str, want: i64, timeout: Duration) -> Duration {
    let start = Instant::now();
    loop {
        let (_, s) = totals(pool, prefix).await;
        if s == want {
            return start.elapsed();
        }
        assert!(start.elapsed() < timeout, "timeout: suma liczników {s}, oczekiwano {want}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Suma zatwierdzonych offsetów grupy na topikach `events.normalized` i `alerts.commands` (po 2 partycje).
fn committed_sum() -> i64 {
    let c: BaseConsumer =
        ClientConfig::new().set("bootstrap.servers", common::brokers()).set("group.id", common::group(GROUP)).create().unwrap();
    let t = common::topics();
    let mut tpl = TopicPartitionList::new();
    for topic in [&t.events_normalized, &t.alerts_commands] {
        for p in 0..2 {
            tpl.add_partition(topic, p);
        }
    }
    let l = c.committed_offsets(tpl, Duration::from_secs(10)).unwrap();
    l.elements().iter().map(|e| if let Offset::Offset(n) = e.offset() { n } else { 0 }).sum()
}

fn fps(prefix: &str, n: usize) -> Vec<String> {
    (0..n).map(|i| format!("{prefix}{i}")).collect()
}

/// Publikuje listę zdarzeń (po fingerprincie) partiami.
async fn publish_all(producer: &FutureProducer, list: &[String]) {
    for chunk in list.chunks(200) {
        let futs = chunk.iter().map(|fp| {
            let ev = common::event(fp, false);
            async move { ::common::kafka::publish(producer, &common::topics().events_normalized, &ev.fingerprint, &ev).await.unwrap() }
        });
        futures::future::join_all(futs).await;
    }
}

/// Wymieszany ruch: `n` fingerprintów × `m` zdarzeń + gorące klucze (`hot` × `hot_n`).
fn mixed_traffic(prefix: &str, n: usize, m: usize, hot: usize, hot_n: usize) -> (Vec<String>, Vec<String>) {
    let mut all_fps = fps(&format!("{prefix}c"), n);
    all_fps.extend(fps(&format!("{prefix}hot"), hot));
    let mut events = Vec::new();
    for i in 0..m.max(hot_n) {
        for (k, fp) in all_fps.iter().enumerate() {
            let limit = if k < n { m } else { hot_n };
            if i < limit {
                events.push(fp.clone());
            }
        }
    }
    (all_fps, events)
}

async fn opened_per_fp(prefix: &str) -> HashMap<String, usize> {
    let all: Vec<AlertChanged> = common::read_topic(
        &common::topics().alerts_changed,
        Duration::from_secs(4),
        |c: &AlertChanged| c.fingerprint.starts_with(prefix) && c.transition == Transition::Opened,
        |_| false,
    )
    .await;
    let mut m = HashMap::new();
    for c in all {
        *m.entry(c.fingerprint).or_insert(0) += 1;
    }
    m
}

/// (a) Kolejność per fingerprint między dwoma topikami, przy zajętych pozostałych pasach.
#[tokio::test]
async fn order_per_fingerprint_across_topics() {
    let _g = SERIAL.lock().await;
    let (pool, producer) = common::setup().await;
    let prefix = format!("dc-ord-{}-", Uuid::new_v4().simple());
    let targets = fps(&format!("{prefix}t"), 8);
    let noise = fps(&format!("{prefix}n"), 40);
    let mut cl = common::Cleanup::new(TESTS);
    targets.iter().chain(noise.iter()).for_each(|f| cl.fingerprint(f));
    let d = spawn_dedup(&pool, &producer);

    // ruch w tle na innych fingerprintach przez cały test
    let noise_task = {
        let (p, noise) = (producer.clone(), noise.clone());
        tokio::spawn(async move {
            for _ in 0..30 {
                publish_all(&p, &noise).await;
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
    };

    let mut flows = Vec::new();
    for fp in targets.clone() {
        let (pool, producer) = (pool.clone(), producer.clone());
        flows.push(tokio::spawn(async move {
            let wait = |what: &'static str, ok: fn(&str, i32, i64) -> bool| {
                let (pool, fp) = (pool.clone(), fp.clone());
                async move {
                    common::wait_for(what, Duration::from_secs(40), || async {
                        let r = sqlx::query("SELECT id, state, episode, count FROM alerts WHERE fingerprint = $1")
                            .bind(&fp)
                            .fetch_optional(&pool)
                            .await
                            .unwrap()?;
                        let (id, st, ep, c): (Uuid, String, i32, i64) = (r.get("id"), r.get("state"), r.get("episode"), r.get("count"));
                        ok(&st, ep, c).then_some(id)
                    })
                    .await
                }
            };
            publish_all(&producer, std::slice::from_ref(&fp)).await;
            let id = wait("open", |s, e, c| s == "open" && e == 1 && c == 1).await;
            common::send_command(&producer, id, &fp, AlertCommandKind::Ack, "p1").await;
            wait("acked", |s, _, _| s == "acked").await;
            publish_all(&producer, std::slice::from_ref(&fp)).await;
            common::send_command(&producer, id, &fp, AlertCommandKind::Resolve, "p1").await;
            wait("resolved", |s, e, _| s == "resolved" && e == 1).await;
            publish_all(&producer, std::slice::from_ref(&fp)).await;
            wait("reopened", |s, e, c| s == "reopened" && e == 2 && c == 1).await;
        }));
    }
    for f in flows {
        f.await.unwrap();
    }
    noise_task.await.unwrap();

    let all: Vec<AlertChanged> = common::read_topic(
        &common::topics().alerts_changed,
        Duration::from_secs(30),
        |c: &AlertChanged| c.fingerprint.starts_with(&format!("{prefix}t")),
        |o: &[AlertChanged]| o.len() >= targets.len() * 4,
    )
    .await;
    for fp in &targets {
        let seq: Vec<Transition> = all.iter().filter(|c| &c.fingerprint == fp).map(|c| c.transition).collect();
        assert_eq!(seq, vec![Transition::Opened, Transition::Acked, Transition::Resolved, Transition::Reopened], "{fp}");
    }
    stop(d).await;
    let (n, _) = totals(&pool, &format!("{prefix}t")).await;
    assert_eq!(n, targets.len() as i64);
}

/// Jeden przebieg (b): produkcja przy zatrzymanym dedupie, potem pomiar czasu konsumpcji.
async fn counters_run(pool: &PgPool, producer: &FutureProducer, cl_fps: &mut Vec<String>, lanes: usize) -> (Duration, Duration) {
    std::env::set_var("DEDUP_LANES", lanes.to_string());
    let prefix = format!("dc-cnt-{}-", Uuid::new_v4().simple());
    let (n, m, hot, hot_n) = (300, 10, 3, 1000);
    let (all_fps, events) = mixed_traffic(&prefix, n, m, hot, hot_n);
    cl_fps.extend(all_fps.iter().cloned());
    publish_all(producer, &events).await;
    let total = events.len() as i64;

    let d = spawn_dedup(pool, producer);
    let took = wait_sum(pool, &prefix, total, Duration::from_secs(180)).await;
    let (alerts, sum) = totals(pool, &prefix).await;
    assert_eq!(alerts, all_fps.len() as i64);
    assert_eq!(sum, total);
    let opened = opened_per_fp(&prefix).await;
    assert_eq!(opened.len(), all_fps.len(), "każdy fingerprint otwarty");
    assert!(opened.values().all(|&c| c == 1), "podwójne Opened: {opened:?}");
    stop(d).await;

    // gorący klucz sam: ruch jednego fingerprintu trafia do jednego pasa
    let hp = format!("dc-hot-{}-", Uuid::new_v4().simple());
    let hot_fp = format!("{hp}0");
    cl_fps.push(hot_fp.clone());
    publish_all(producer, &vec![hot_fp; 6000]).await;
    let d = spawn_dedup(pool, producer);
    let hot_took = wait_sum(pool, &hp, 6000, Duration::from_secs(120)).await;
    stop(d).await;
    println!("lanes={lanes}: mixed {total} events in {took:?} ({:.0} msg/s); hot-only 6000 in {hot_took:?}", total as f64 / took.as_secs_f64());
    (took, hot_took)
}

/// (b)+(5) Poprawność liczników pod współbieżnością, czas 1 pas vs domyślnie; gorący klucz.
#[tokio::test]
async fn counters_correct_under_concurrency() {
    let _g = SERIAL.lock().await;
    let (pool, producer) = common::setup().await;
    let mut fps_all = Vec::new();
    let mut cl = common::Cleanup::new(TESTS);
    let one = counters_run(&pool, &producer, &mut fps_all, 1).await;
    let many = counters_run(&pool, &producer, &mut fps_all, 0).await;
    fps_all.iter().for_each(|f| cl.fingerprint(f));
    std::env::remove_var("DEDUP_LANES");
    println!("TIMING 1 lane: mixed {:?}, hot {:?}; default lanes: mixed {:?}, hot {:?}", one.0, one.1, many.0, many.1);
}

/// (c) Rebalans: druga instancja dołącza w trakcie strumienia, potem pierwsza się zatrzymuje.
#[tokio::test]
async fn rebalance_keeps_counters_exact() {
    let _g = SERIAL.lock().await;
    let (pool, producer) = common::setup().await;
    let prefix = format!("dc-reb-{}-", Uuid::new_v4().simple());
    let list = fps(&prefix, 30);
    let mut cl = common::Cleanup::new(TESTS);
    list.iter().for_each(|f| cl.fingerprint(f));

    let mut a = Some(spawn_dedup(&pool, &producer));
    let mut sent = 0i64;
    let mut b = None;
    for round in 0..40 {
        for _ in 0..10 {
            publish_all(&producer, &list).await;
            sent += list.len() as i64;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
        if round == 10 {
            b = Some(spawn_dedup(&pool, &producer));
        }
        if round == 25 {
            stop(a.take().unwrap()).await;
        }
    }
    wait_sum(&pool, &prefix, sent, Duration::from_secs(90)).await;
    let (n, s) = totals(&pool, &prefix).await;
    assert_eq!((n, s), (list.len() as i64, sent));
    let opened = opened_per_fp(&prefix).await;
    assert!(opened.values().all(|&c| c == 1), "podwójne Opened: {opened:?}");
    stop(b.take().unwrap()).await;
}

/// (d) Shutdown: flush liczników + commit offsetów.
#[tokio::test]
async fn shutdown_flushes_and_commits() {
    let _g = SERIAL.lock().await;
    let (pool, producer) = common::setup().await;
    let prefix = format!("dc-shut-{}-", Uuid::new_v4().simple());
    let list = fps(&prefix, 20);
    let mut cl = common::Cleanup::new(TESTS);
    list.iter().for_each(|f| cl.fingerprint(f));
    let base = committed_sum();
    let mut events = Vec::new();
    for _ in 0..50 {
        events.extend(list.iter().cloned());
    }
    publish_all(&producer, &events).await;
    let total = events.len() as i64;

    let d = spawn_dedup(&pool, &producer);
    common::wait_for("commit", Duration::from_secs(40), || async { (committed_sum() - base == total).then_some(()) }).await;
    stop(d).await;
    let (n, s) = totals(&pool, &prefix).await;
    assert_eq!((n, s), (list.len() as i64, total), "po shutdownie liczniki kompletne");
    assert_eq!(committed_sum() - base, total);
}
