//! Testy integracyjne współbieżnego routera (broker Kafki z docker compose, izolowane topiki/grupy):
//! przepustowość, kolejność per alert, commit offsetów po skutkach.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};

use common::kafka::rdkafka::admin::{AdminClient, AdminOptions, NewTopic, TopicReplication};
use common::kafka::rdkafka::client::DefaultClientContext;
use common::kafka::rdkafka::config::ClientConfig;
use common::kafka::rdkafka::consumer::{Consumer, StreamConsumer};
use common::kafka::rdkafka::message::Message;
use common::kafka::rdkafka::{Offset, TopicPartitionList};
use common::kafka::ConcurrentConfig;
use contracts::{AlertChanged, DeliveryRequest};
use router::{
    parse_and_validate_rule_yaml, route_alert, run_router_on_topics, CompiledRule,
    PeopleConfigFile, RouterSnapshot,
};
use uuid::Uuid;

const IN_PARTITIONS: i32 = 4;
const UI_URL: &str = "http://localhost:5173";

fn brokers() -> String {
    std::env::var("KAFKA_BROKERS").unwrap_or_else(|_| "localhost:19092".into())
}

fn repo_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn snapshot() -> RouterSnapshot {
    let root = repo_root();
    let people: PeopleConfigFile =
        serde_yaml::from_str(&std::fs::read_to_string(root.join("config/people.yaml")).unwrap())
            .unwrap();
    let mut rules = Vec::new();
    let mut paths: Vec<_> = std::fs::read_dir(root.join("config/rules"))
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| matches!(p.extension().and_then(|s| s.to_str()), Some("yaml" | "yml")))
        .collect();
    paths.sort();
    for p in paths {
        let def = parse_and_validate_rule_yaml(&std::fs::read_to_string(p).unwrap()).unwrap();
        rules.push(CompiledRule::compile(def).unwrap());
    }
    RouterSnapshot {
        rules,
        people: people
            .people
            .into_iter()
            .map(|p| (p.id.clone(), p))
            .collect(),
        teams: people
            .teams
            .into_iter()
            .map(|t| (t.id.clone(), t))
            .collect(),
        manual_runbooks: vec![],
    }
}

fn alert_template() -> AlertChanged {
    let raw = std::fs::read_to_string(
        repo_root().join("crates/contracts/fixtures/topics/alerts.changed.json"),
    )
    .unwrap();
    serde_json::from_str(&raw).unwrap()
}

struct Env {
    id: String,
    topic_in: String,
    topic_out: String,
    group: String,
}

async fn setup() -> Env {
    let id = Uuid::new_v4().simple().to_string();
    let env = Env {
        topic_in: format!("rt-changed-{id}"),
        topic_out: format!("rt-deliveries-{id}"),
        group: format!("rt-group-{id}"),
        id,
    };
    let admin: AdminClient<DefaultClientContext> = ClientConfig::new()
        .set("bootstrap.servers", brokers())
        .create()
        .unwrap();
    let topics = [
        NewTopic::new(&env.topic_in, IN_PARTITIONS, TopicReplication::Fixed(1)),
        NewTopic::new(&env.topic_out, 3, TopicReplication::Fixed(1)),
    ];
    for r in admin
        .create_topics(topics.iter(), &AdminOptions::new())
        .await
        .unwrap()
    {
        r.unwrap();
    }
    env
}

async fn teardown(env: &Env) {
    let admin: AdminClient<DefaultClientContext> = ClientConfig::new()
        .set("bootstrap.servers", brokers())
        .create()
        .unwrap();
    let _ = admin
        .delete_topics(
            &[env.topic_in.as_str(), env.topic_out.as_str()],
            &AdminOptions::new(),
        )
        .await;
}

fn store_with(snap: RouterSnapshot) -> router::RouterStore {
    let store = router::new_store();
    store.store(std::sync::Arc::new(snap));
    store
}

/// Uruchamia router w tle; zwraca uchwyt zadania i nadajnik shutdownu.
fn spawn_router(
    env: &Env,
    lanes: usize,
) -> (
    tokio::task::JoinHandle<anyhow::Result<()>>,
    tokio::sync::watch::Sender<bool>,
) {
    let (tx, rx) = tokio::sync::watch::channel(false);
    let producer = common::kafka::producer(&brokers()).unwrap();
    let mut cfg = ConcurrentConfig::from_env("ROUTER_IT", lanes);
    cfg.lanes = lanes;
    cfg.max_in_flight = (8 * lanes).max(64);
    let (group, tin, tout) = (
        env.group.clone(),
        env.topic_in.clone(),
        env.topic_out.clone(),
    );
    let store = store_with(snapshot());
    let task = tokio::spawn(async move {
        run_router_on_topics(
            &group,
            &brokers(),
            &tin,
            &tout,
            UI_URL.into(),
            producer,
            store,
            cfg,
            rx,
        )
        .await
    });
    (task, tx)
}

async fn publish_changes(env: &Env, alerts: &[AlertChanged]) {
    let producer = common::kafka::producer(&brokers()).unwrap();
    let keys: Vec<String> = alerts.iter().map(|a| a.alert_id.to_string()).collect();
    let sends = alerts
        .iter()
        .zip(&keys)
        .map(|(a, k)| common::kafka::publish(&producer, &env.topic_in, k, a));
    futures::future::try_join_all(sends).await.unwrap();
}

/// Czyta z topiku wyjściowego, aż zbierze `expected` zleceń (lub timeout).
async fn collect_output(env: &Env, expected: usize, timeout: Duration) -> Vec<DeliveryRequest> {
    let consumer =
        common::kafka::consumer(&brokers(), &format!("rt-out-{}", env.id), &[&env.topic_out])
            .unwrap();
    let start = Instant::now();
    let mut out = Vec::new();
    while out.len() < expected && start.elapsed() < timeout {
        if let Ok(Ok(m)) = tokio::time::timeout(Duration::from_millis(500), consumer.recv()).await {
            out.push(serde_json::from_slice(m.payload().unwrap()).unwrap());
        }
    }
    out
}

fn committed_total(env: &Env) -> i64 {
    let check: StreamConsumer = ClientConfig::new()
        .set("bootstrap.servers", brokers())
        .set("group.id", &env.group)
        .set("enable.auto.commit", "false")
        .create()
        .unwrap();
    let mut tpl = TopicPartitionList::new();
    for p in 0..IN_PARTITIONS {
        tpl.add_partition_offset(&env.topic_in, p, Offset::Invalid)
            .unwrap();
    }
    check
        .committed_offsets(tpl, Duration::from_secs(10))
        .unwrap()
        .elements()
        .iter()
        .map(|e| match e.offset() {
            Offset::Offset(o) => o,
            _ => 0,
        })
        .sum()
}

fn distinct_alerts(n: usize) -> Vec<AlertChanged> {
    (0..n)
        .map(|_| {
            let mut a = alert_template();
            a.alert_id = Uuid::new_v4();
            a
        })
        .collect()
}

/// Mierzy czas od startu routera (wiadomości już opublikowane) do skompletowania wszystkich zleceń.
async fn run_batch(n: usize, lanes: usize) -> (Duration, Vec<DeliveryRequest>, Env) {
    let env = setup().await;
    let alerts = distinct_alerts(n);
    let per_alert = route_alert(&snapshot(), &alerts[0], UI_URL).len();
    assert!(per_alert > 0, "fixture musi generować zlecenia");
    publish_changes(&env, &alerts).await;

    let start = Instant::now();
    let (task, tx) = spawn_router(&env, lanes);
    let out = collect_output(&env, n * per_alert, Duration::from_secs(120)).await;
    let elapsed = start.elapsed();
    tx.send(true).unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(
        committed_total(&env),
        n as i64,
        "offsety skommitowane po skutkach (shutdown)"
    );
    (elapsed, out, env)
}

#[tokio::test(flavor = "multi_thread")]
async fn publishes_all_deliveries_faster_than_sequential_and_commits_after_effects() {
    let per_alert = route_alert(&snapshot(), &alert_template(), UI_URL).len();

    let (seq_time, seq_out, seq_env) = run_batch(200, 1).await;
    assert_eq!(seq_out.len(), 200 * per_alert);
    let seq_rate = 200.0 / seq_time.as_secs_f64();

    let (time, out, env) = run_batch(1000, 32).await;
    assert_eq!(
        out.len(),
        1000 * per_alert,
        "wszystkie zlecenia opublikowane"
    );
    let ids: HashSet<Uuid> = out.iter().map(|r| r.alert_id).collect();
    assert_eq!(ids.len(), 1000);
    let rate = 1000.0 / time.as_secs_f64();
    println!(
        "THROUGHPUT: 1 pas: 200 alertów w {:.2}s ({seq_rate:.0}/s); 32 pasy: 1000 alertów ({} zleceń) w {:.2}s ({rate:.0}/s)",
        seq_time.as_secs_f64(),
        out.len(),
        time.as_secs_f64()
    );
    assert!(
        rate > 2.0 * seq_rate,
        "współbieżny router ma być wyraźnie szybszy: {rate:.0}/s vs {seq_rate:.0}/s"
    );
    teardown(&seq_env).await;
    teardown(&env).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn preserves_change_order_per_alert() {
    let env = setup().await;
    let alerts_n = 20;
    let changes_per_alert = 15;
    let base: Vec<AlertChanged> = distinct_alerts(alerts_n);
    let mut msgs = Vec::new();
    for ep in 1..=changes_per_alert {
        for a in &base {
            let mut c = a.clone();
            c.episode = ep;
            msgs.push(c);
        }
    }
    let per_change = route_alert(&snapshot(), &msgs[0], UI_URL).len();
    let producer = common::kafka::producer(&brokers()).unwrap();
    for m in &msgs {
        common::kafka::publish(&producer, &env.topic_in, &m.alert_id.to_string(), m)
            .await
            .unwrap();
    }

    let (task, tx) = spawn_router(&env, 16);
    let out = collect_output(&env, msgs.len() * per_change, Duration::from_secs(60)).await;
    tx.send(true).unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(out.len(), msgs.len() * per_change);

    let mut per_alert: HashMap<Uuid, Vec<i32>> = HashMap::new();
    for r in &out {
        per_alert.entry(r.alert_id).or_default().push(r.episode);
    }
    assert_eq!(per_alert.len(), alerts_n);
    for (id, eps) in per_alert {
        assert!(
            eps.windows(2).all(|w| w[0] <= w[1]),
            "kolejność naruszona dla {id}: {eps:?}"
        );
    }
    assert_eq!(committed_total(&env), msgs.len() as i64);
    teardown(&env).await;
}
