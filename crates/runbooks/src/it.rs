//! Testy integracyjne konsumenta `runbooks` na prawdziwym Postgresie i Kafce (izolowane topiki i grupa).
//!
//! Wymagają `DATABASE_URL` (migrowanej bazy) i `KAFKA_BROKERS` (domyślnie `localhost:19092`).
//! Benchmark: `DB_MAX_CONNECTIONS=6 cargo test -p runbooks --release bench -- --ignored --nocapture`.
//!
//! WYNIKI POMIARU (konsument sekwencyjny, lokalny Postgres 16 + Redpanda, pula PG = 6; czas do skommitowania
//! offsetów 2000 wiadomości na 4 partycjach, 3 powtórzenia, w tym start grupy ~0,4 s):
//! - 2000 bez dopasowania: 0,37-0,94 s (2,1-5,4 tys./s), zero zapytań do PG (tylko migawka w pamięci, CPU),
//! - 2000, 5% pasuje: 1,0-1,4 s (1,4-2 tys./s),
//! - 400, wszystkie pasują: 0,9-2,1 s (190-450/s); ~5 round-tripów PG w `create_run` (BEGIN, INSERT run,
//!   INSERT per krok, INSERT audit, COMMIT) + publikacja `ui.events`, a w tle `advance` (~15 zapytań/run).
//!
//! Współbieżne pasy (`run_concurrent`, 1/4/8/16 pasów, pula 6 i 10) NIE dały mierzalnego zysku (różnice w
//! szumie ±50%), a przy puli 6 i 16 pasach przepustowość spadła (90-120/s dla wszystkich pasujących):
//! wąskim gardłem jest pula PG współdzielona z odłączonymi zadaniami `advance`, nie czekanie konsumenta.

use std::sync::Arc;
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use common::db::sqlx::{self, PgPool};
use common::kafka::rdkafka::admin::{AdminClient, AdminOptions, NewTopic, TopicReplication};
use common::kafka::rdkafka::client::DefaultClientContext;
use common::kafka::rdkafka::config::ClientConfig;
use common::kafka::rdkafka::consumer::{Consumer, StreamConsumer};
use common::kafka::rdkafka::{Offset, TopicPartitionList};
use contracts::{Actor, AlertChanged, RunbookCommand, RunbookCommandKind, Transition};
use uuid::Uuid;

use crate::engine::Engine;
use crate::parse::parse_runbook;
use crate::registry::{Executor, Registry};
use crate::store::Snapshot;

const PARTITIONS: i32 = 4;

fn brokers() -> String {
    std::env::var("KAFKA_BROKERS").unwrap_or_else(|_| "localhost:19092".into())
}

fn alert_template() -> AlertChanged {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../contracts/fixtures/topics/alerts.changed.json"
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// Alert pasujący (`WORKFLOW_FAILED` + `TIMEOUT`) albo nie (inny typ i kod) do runbooka testowego.
fn alert(matching: bool) -> AlertChanged {
    let mut a = alert_template();
    a.alert_id = Uuid::new_v4();
    if !matching {
        a.event.fields.insert("type".into(), "OTHER".into());
        a.event.raw["type"] = "OTHER".into();
        a.event.raw["error"]["code"] = "OK".into();
    }
    a
}

struct Env {
    changed: String,
    commands: String,
    group: String,
    pool: PgPool,
    engine: Arc<Engine>,
}

/// Usługa docelowa akcji `http_get` (zawsze 200) na losowym porcie.
async fn target() -> String {
    use axum::{routing::get, Router};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new().route("/health", get(|| async { "ok" }));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    base
}

/// `steps_yaml`: kroki runbooka testowego dopasowanego do alertów `alert(true)`.
async fn setup(steps: impl Fn(&str) -> String) -> Env {
    let id = Uuid::new_v4().simple().to_string();
    let admin: AdminClient<DefaultClientContext> = ClientConfig::new()
        .set("bootstrap.servers", brokers())
        .create()
        .unwrap();
    let (changed, commands) = (format!("rb-changed-{id}"), format!("rb-commands-{id}"));
    let topics = [
        NewTopic::new(&changed, PARTITIONS, TopicReplication::Fixed(1)),
        NewTopic::new(&commands, PARTITIONS, TopicReplication::Fixed(1)),
    ];
    for r in admin
        .create_topics(topics.iter(), &AdminOptions::new())
        .await
        .unwrap()
    {
        r.unwrap();
    }

    let base = target().await;
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let pool = common::db::connect_and_migrate_with(&url, crate::DB_POOL_DEFAULT)
        .await
        .unwrap();
    let registry = Registry::new(vec![]);
    let def = parse_runbook(
        "bench.yaml",
        &format!("runbook: bench-{id}\ntitle: Bench\nmatches: {{ type: WORKFLOW_FAILED, \"error.code\": TIMEOUT }}\nsteps:\n{}", steps(&base)),
    )
    .unwrap();
    let mut runbooks = vec![registry.classify(&def)];
    for file in [
        "config/runbooks/payments-high-error-rate.md",
        "config/runbooks/pricing-latency.txt",
        "config/runbooks/cdn-stale.yaml",
    ] {
        let text = std::fs::read_to_string(format!("{}/../../{file}", env!("CARGO_MANIFEST_DIR")))
            .unwrap();
        runbooks.push(registry.classify(&parse_runbook(file, &text).unwrap()));
    }
    for a in [alert(true), alert(false)] {
        let n = crate::matching::matching(&runbooks, &[], &a).len();
        assert_eq!(
            n,
            usize::from(a.event.fields["type"] == "WORKFLOW_FAILED"),
            "dopasowanie runbooków testu"
        );
    }
    let engine = Arc::new(Engine {
        pool: pool.clone(),
        producer: common::kafka::producer(&brokers()).unwrap(),
        executor: Executor::new(vec!["127.0.0.1".into()]),
        snapshot: Arc::new(ArcSwap::from_pointee(Snapshot {
            registry,
            runbooks,
            ..Default::default()
        })),
        public_ui_url: "http://localhost:5173".into(),
        active_runs: Default::default(),
    });
    Env {
        group: format!("rb-group-{id}"),
        changed,
        commands,
        pool,
        engine,
    }
}

async fn teardown(env: &Env) {
    let admin: AdminClient<DefaultClientContext> = ClientConfig::new()
        .set("bootstrap.servers", brokers())
        .create()
        .unwrap();
    let _ = admin
        .delete_topics(
            &[env.changed.as_str(), env.commands.as_str()],
            &AdminOptions::new(),
        )
        .await;
}

/// Wysyła wszystkie wiadomości współbieżnie, ale kolejność per klucz zachowuje (jedno zadanie na grupę kluczy
/// nie jest potrzebne: `acks=all` + idempotentny producent i `enqueue` w kolejności wywołań gwarantują
/// kolejność per partycja, a `JoinSet` tylko czeka na potwierdzenia).
async fn publish<T: serde::Serialize>(topic: &str, items: &[(String, T)]) {
    let producer = common::kafka::producer(&brokers()).unwrap();
    let mut set = tokio::task::JoinSet::new();
    for (key, item) in items {
        let payload = serde_json::to_vec(item).unwrap();
        let (producer, topic, key) = (producer.clone(), topic.to_string(), key.clone());
        let fut = producer
            .send_result(
                common::kafka::rdkafka::producer::FutureRecord::to(&topic)
                    .key(&key)
                    .payload(&payload),
            )
            .map_err(|(e, _)| e)
            .unwrap();
        set.spawn(async move { fut.await.unwrap().unwrap() });
    }
    while let Some(r) = set.join_next().await {
        r.unwrap();
    }
}

fn changed_msgs(alerts: &[AlertChanged]) -> Vec<(String, AlertChanged)> {
    alerts
        .iter()
        .map(|a| (a.alert_id.to_string(), a.clone()))
        .collect()
}

/// Sumaryczny commitowany offset grupy na topikach wejściowych.
fn committed_total(env: &Env) -> i64 {
    let check: StreamConsumer = ClientConfig::new()
        .set("bootstrap.servers", brokers())
        .set("group.id", &env.group)
        .set("enable.auto.commit", "false")
        .create()
        .unwrap();
    let mut tpl = TopicPartitionList::new();
    for topic in [&env.changed, &env.commands] {
        for p in 0..PARTITIONS {
            tpl.add_partition_offset(topic, p, Offset::Invalid).unwrap();
        }
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

async fn wait_until<F, Fut>(timeout: Duration, what: &str, mut cond: F) -> Duration
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let start = Instant::now();
    while !cond().await {
        assert!(start.elapsed() < timeout, "timeout czekania na: {what}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    start.elapsed()
}

async fn count(env: &Env, sql: &str, ids: &[Uuid]) -> i64 {
    sqlx::query_scalar(sql)
        .bind(ids)
        .fetch_one(&env.pool)
        .await
        .unwrap()
}

/// Statystyki PG są flushowane z opóźnieniem (~1 s), więc czekamy przed odczytem.
async fn xacts(pool: &PgPool) -> i64 {
    tokio::time::sleep(Duration::from_millis(1500)).await;
    sqlx::query_scalar("SELECT xact_commit + xact_rollback FROM pg_stat_database WHERE datname = current_database()").fetch_one(pool).await.unwrap()
}

const SIMPLE_STEPS: fn(&str) -> String =
    |base| format!("  - {{check: health, action: http_get, url: '{base}/health'}}\n");

/// Pomiar: `n` zmian alertów, co `every`-ta pasuje do runbooka (`0` = żadna). Zwraca (czas do skommitowania
/// wszystkich offsetów, czas do ukończenia wszystkich runów, liczba transakcji PG).
async fn measure(n: usize, every: usize) -> (Duration, Duration, i64, usize) {
    let env = setup(SIMPLE_STEPS).await;
    let alerts: Vec<AlertChanged> = (0..n).map(|i| alert(every > 0 && i % every == 0)).collect();
    let matching: Vec<Uuid> = alerts
        .iter()
        .enumerate()
        .filter(|(i, _)| every > 0 && i % every == 0)
        .map(|(_, a)| a.alert_id)
        .collect();
    publish(&env.changed, &changed_msgs(&alerts)).await;

    let x0 = xacts(&env.pool).await;
    let start = Instant::now();
    let consumer = start_consumer(&env);
    let drained = wait_until(
        Duration::from_secs(300),
        "skommitowanie wszystkich offsetów",
        || async { committed_total(&env) >= n as i64 },
    )
    .await;
    wait_until(Duration::from_secs(300), "ukończenie runów", || async {
        count(
            &env,
            "SELECT count(*) FROM runbook_runs WHERE alert_id = ANY($1) AND status = 'completed'",
            &matching,
        )
        .await
            == matching.len() as i64
    })
    .await;
    let total = start.elapsed();
    let x1 = xacts(&env.pool).await;
    consumer.stop().await;
    teardown(&env).await;
    (drained, total, x1 - x0, matching.len())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "benchmark ręczny: --ignored --nocapture (RUNBOOKS_LANES, DB_MAX_CONNECTIONS)"]
async fn bench_consumer_throughput() {
    let n = 2000;
    for (label, n, every) in [
        ("bez dopasowania", n, 0),
        ("mix 5% pasuje", n, 20),
        ("wszystkie pasują", 400, 1),
    ] {
        let mut rates = Vec::new();
        for _ in 0..3 {
            let (drained, total, x, runs) = measure(n, every).await;
            rates.push(format!(
                "{:.2}s ({:.0}/s; całość {:.2}s)",
                drained.as_secs_f64(),
                n as f64 / drained.as_secs_f64(),
                total.as_secs_f64()
            ));
            let _ = (x, runs);
        }
        println!(
            "BENCH {n} {label}: offsety skommitowane po: {}",
            rates.join(" | ")
        );
    }
}

/// Dwie komendy dla tego samego runu wysłane jedna po drugiej (`reject`, potem `approve`) muszą zostać
/// zastosowane w tej kolejności: krok kończy `rejected`, a `approve` jest ignorowane.
#[tokio::test(flavor = "multi_thread")]
async fn commands_for_one_run_apply_in_order() {
    let env = setup(|base| format!("  - {{action: http_get, url: '{base}/health'}}\n  - {{action: http_get, url: '{base}/health', approval: required}}\n")).await;
    let alerts: Vec<AlertChanged> = (0..40).map(|_| alert(true)).collect();
    let ids: Vec<Uuid> = alerts.iter().map(|a| a.alert_id).collect();
    publish(&env.changed, &changed_msgs(&alerts)).await;
    let consumer = start_consumer(&env);
    wait_until(Duration::from_secs(60), "runy czekają na zatwierdzenie", || async {
        count(&env, "SELECT count(*) FROM runbook_runs r JOIN step_runs s USING (run_id) WHERE r.alert_id = ANY($1) AND s.step_id = 's2' AND s.status = 'waiting_approval'", &ids).await == 40
    })
    .await;

    let runs: Vec<(Uuid, Uuid)> =
        sqlx::query_as("SELECT alert_id, run_id FROM runbook_runs WHERE alert_id = ANY($1)")
            .bind(&ids)
            .fetch_all(&env.pool)
            .await
            .unwrap();
    let cmd = |alert_id, run_id, kind| RunbookCommand {
        alert_id,
        run_id,
        step_id: "s2".into(),
        kind,
        actor: Actor::User {
            person_id: "jan".into(),
        },
        at: chrono::Utc::now(),
    };
    let mut msgs = Vec::new();
    for (alert_id, run_id) in &runs {
        msgs.push((
            alert_id.to_string(),
            cmd(*alert_id, *run_id, RunbookCommandKind::Reject),
        ));
        msgs.push((
            alert_id.to_string(),
            cmd(*alert_id, *run_id, RunbookCommandKind::Approve),
        ));
    }
    publish(&env.commands, &msgs).await;
    wait_until(Duration::from_secs(60), "komendy przetworzone", || async {
        committed_total(&env) >= 40 + 80
    })
    .await;
    let rejected = count(&env, "SELECT count(*) FROM runbook_runs r JOIN step_runs s USING (run_id) WHERE r.alert_id = ANY($1) AND s.step_id = 's2' AND s.status = 'rejected'", &ids).await;
    let approved = count(
        &env,
        "SELECT count(*) FROM audit_log WHERE alert_id = ANY($1) AND action = 'step_approved'",
        &ids,
    )
    .await;
    consumer.stop().await;
    teardown(&env).await;
    assert_eq!(
        (rejected, approved),
        (40, 0),
        "reject poprzedza approve dla każdego runu"
    );
}

/// Powtórzenie tej samej zmiany alertu (redelivery) nie tworzy drugiego runa dla (alert, epizod, runbook).
#[tokio::test(flavor = "multi_thread")]
async fn redelivered_alert_change_creates_one_run() {
    let env = setup(SIMPLE_STEPS).await;
    let alerts: Vec<AlertChanged> = (0..20).map(|_| alert(true)).collect();
    let ids: Vec<Uuid> = alerts.iter().map(|a| a.alert_id).collect();
    let mut msgs = changed_msgs(&alerts);
    msgs.extend(changed_msgs(&alerts));
    msgs.extend(changed_msgs(&alerts));
    let acked: Vec<AlertChanged> = alerts
        .iter()
        .cloned()
        .map(|a| AlertChanged {
            transition: Transition::Acked,
            ..a
        })
        .collect();
    msgs.extend(changed_msgs(&acked));
    publish(&env.changed, &msgs).await;
    let consumer = start_consumer(&env);
    wait_until(
        Duration::from_secs(60),
        "wszystkie wiadomości skommitowane",
        || async { committed_total(&env) >= msgs.len() as i64 },
    )
    .await;
    wait_until(Duration::from_secs(60), "runy ukończone", || async {
        count(
            &env,
            "SELECT count(*) FROM runbook_runs WHERE alert_id = ANY($1) AND status = 'completed'",
            &ids,
        )
        .await
            == 20
    })
    .await;
    let runs = count(
        &env,
        "SELECT count(*) FROM runbook_runs WHERE alert_id = ANY($1)",
        &ids,
    )
    .await;
    let started = count(
        &env,
        "SELECT count(*) FROM audit_log WHERE alert_id = ANY($1) AND action = 'run_started'",
        &ids,
    )
    .await;
    let steps = count(&env, "SELECT count(*) FROM step_runs s JOIN runbook_runs r USING (run_id) WHERE r.alert_id = ANY($1)", &ids).await;
    consumer.stop().await;
    teardown(&env).await;
    assert_eq!((runs, started, steps), (20, 20, 20));
}

/// Offset jest commitowany dopiero po zapisaniu skutków: po zatrzymaniu zacommitowane = wszystko,
/// a każda zacommitowana wiadomość pasująca do runbooka ma już run w bazie.
#[tokio::test(flavor = "multi_thread")]
async fn offsets_are_committed_after_effects_and_survive_shutdown() {
    let env = setup(SIMPLE_STEPS).await;
    let alerts: Vec<AlertChanged> = (0..300).map(|i| alert(i % 3 == 0)).collect();
    let matching: Vec<Uuid> = alerts
        .iter()
        .enumerate()
        .filter(|(i, _)| i % 3 == 0)
        .map(|(_, a)| a.alert_id)
        .collect();
    publish(&env.changed, &changed_msgs(&alerts)).await;
    let consumer = start_consumer(&env);
    wait_until(Duration::from_secs(60), "start przetwarzania", || async {
        committed_total(&env) > 0
    })
    .await;
    consumer.stop().await;
    let committed = committed_total(&env);
    let runs = count(
        &env,
        "SELECT count(*) FROM runbook_runs WHERE alert_id = ANY($1)",
        &matching,
    )
    .await;
    assert!(committed > 0);
    let consumer = start_consumer(&env);
    wait_until(Duration::from_secs(60), "wszystko skommitowane", || async {
        committed_total(&env) >= 300
    })
    .await;
    consumer.stop().await;
    let runs_after = count(
        &env,
        "SELECT count(*) FROM runbook_runs WHERE alert_id = ANY($1)",
        &matching,
    )
    .await;
    assert_eq!(committed_total(&env), 300);
    assert_eq!(runs_after, matching.len() as i64);
    assert!(runs <= runs_after);
    teardown(&env).await;
}

/// Uruchomiony konsument testowy (obecny konsument jest sekwencyjny: stop = przerwanie zadania;
/// offset jest commitowany po każdej wiadomości, więc nic nie czeka na flush przy zatrzymaniu).
struct Running {
    task: tokio::task::JoinHandle<anyhow::Result<()>>,
}

impl Running {
    async fn stop(self) {
        tokio::time::sleep(Duration::from_millis(300)).await;
        self.task.abort();
        let _ = self.task.await;
    }
}

fn start_consumer(env: &Env) -> Running {
    let engine = env.engine.clone();
    let (b, g, c, k) = (
        brokers(),
        env.group.clone(),
        env.changed.clone(),
        env.commands.clone(),
    );
    Running {
        task: tokio::spawn(async move { engine.consume_on(&b, &g, &c, &k).await }),
    }
}
