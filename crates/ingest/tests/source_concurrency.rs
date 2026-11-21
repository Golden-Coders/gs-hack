//! Testy współbieżnego konsumenta źródła Kafki (wymagają Kafki `localhost:19092` i Postgresa).
//! Źródło działa na izolowanym topiku (12 partycji); wyniki czytamy z `events.raw` filtrując po unikalnym `schema`.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};

use common::kafka::{
    self,
    rdkafka::{
        admin::{AdminClient, AdminOptions, NewTopic, TopicReplication},
        client::DefaultClientContext,
        config::ClientConfig,
        consumer::{Consumer, StreamConsumer},
        producer::{FutureProducer, FutureRecord},
        Offset, TopicPartitionList,
    },
};
use contracts::{topics, Envelope};
use ingest::sources::{self, Ctx, SourceDef};
use tokio::{sync::watch, task::JoinHandle};
use uuid::Uuid;

const BROKERS: &str = "localhost:19092";
const DB: &str = "postgres://alerting:alerting@localhost:5432/alerting";
const PARTS: i32 = 12;

struct Env {
    topic: String,
    schema: String,
    name: String,
    group: String,
    producer: FutureProducer,
    ctx: Arc<Ctx>,
}

impl Env {
    async fn new() -> Self {
        let id = Uuid::new_v4().simple().to_string();
        let topic = format!("test.srcconc.{id}");
        let admin: AdminClient<DefaultClientContext> = ClientConfig::new()
            .set("bootstrap.servers", BROKERS)
            .create()
            .unwrap();
        admin
            .create_topics(
                &[NewTopic::new(&topic, PARTS, TopicReplication::Fixed(1))],
                &AdminOptions::new(),
            )
            .await
            .unwrap();
        let producer = kafka::producer(BROKERS).unwrap();
        let pool = common::db::sqlx::PgPool::connect(DB).await.unwrap();
        let ctx = Arc::new(Ctx {
            brokers: BROKERS.into(),
            pool,
            producer: producer.clone(),
        });
        let name = format!("t{id}");
        Self {
            schema: format!("t.srcconc.{id}"),
            group: format!("ingest-source-{name}"),
            name,
            topic,
            producer,
            ctx,
        }
    }

    fn def(&self) -> SourceDef {
        SourceDef {
            topic: self.topic.clone(),
            schema: self.schema.clone(),
            format: "json".into(),
        }
    }

    fn start(&self) -> (watch::Sender<bool>, JoinHandle<()>) {
        let (tx, rx) = watch::channel(false);
        (
            tx,
            tokio::spawn(sources::run_source(
                self.ctx.clone(),
                self.name.clone(),
                self.def(),
                rx,
            )),
        )
    }

    /// Wysyła wiadomości `i` z kluczem `k{i % keys}`; w obrębie klucza sekwencyjnie (kolejność w topiku = rosnące `i`),
    /// klucze równolegle.
    async fn send(&self, range: std::ops::Range<usize>, keys: usize) {
        let mut tasks = Vec::new();
        for k in 0..keys {
            let ids: Vec<usize> = range.clone().filter(|i| i % keys == k).collect();
            let (p, topic) = (self.producer.clone(), self.topic.clone());
            tasks.push(tokio::spawn(async move {
                for i in ids {
                    let payload = format!(r#"{{"i":{i}}}"#);
                    p.send(
                        FutureRecord::to(&topic)
                            .key(&format!("k{k}"))
                            .payload(&payload),
                        Duration::from_secs(30),
                    )
                    .await
                    .map_err(|(e, _)| e)
                    .unwrap();
                }
            }));
        }
        for t in tasks {
            t.await.unwrap();
        }
    }

    /// Suma committed offsetów grupy po wszystkich partycjach.
    fn committed_sum(&self) -> i64 {
        let c: StreamConsumer = ClientConfig::new()
            .set("bootstrap.servers", BROKERS)
            .set("group.id", &self.group)
            .set("enable.auto.commit", "false")
            .create()
            .unwrap();
        let mut tpl = TopicPartitionList::new();
        for p in 0..PARTS {
            tpl.add_partition_offset(&self.topic, p, Offset::Invalid)
                .unwrap();
        }
        let committed = c.committed_offsets(tpl, Duration::from_secs(10)).unwrap();
        committed
            .elements()
            .iter()
            .map(|e| {
                if let Offset::Offset(o) = e.offset() {
                    o
                } else {
                    0
                }
            })
            .sum()
    }

    async fn error_count(&self) -> i64 {
        common::db::sqlx::query_scalar("SELECT count(*) FROM ingest_errors WHERE source = $1")
            .bind(format!("kafka:{}", self.topic))
            .fetch_one(&self.ctx.pool)
            .await
            .unwrap()
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let (brokers, topic) = (BROKERS.to_string(), self.topic.clone());
        let pool = self.ctx.pool.clone();
        let source = format!("kafka:{}", self.topic);
        let _ = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async {
                let admin: AdminClient<DefaultClientContext> = ClientConfig::new()
                    .set("bootstrap.servers", brokers)
                    .create()
                    .unwrap();
                let _ = admin.delete_topics(&[&topic], &AdminOptions::new()).await;
                let _ = common::db::sqlx::query("DELETE FROM ingest_errors WHERE source = $1")
                    .bind(source)
                    .execute(&pool)
                    .await;
            });
        })
        .join();
    }
}

/// Konsument `events.raw` startujący od końca; czeka na przypisanie, by nie zgubić wiadomości testu.
async fn raw_tail() -> StreamConsumer {
    let c: StreamConsumer = ClientConfig::new()
        .set("bootstrap.servers", BROKERS)
        .set("group.id", format!("test-{}", Uuid::new_v4()))
        .set("enable.auto.commit", "false")
        .set("auto.offset.reset", "latest")
        .create()
        .unwrap();
    c.subscribe(&[topics::EVENTS_RAW]).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while c.assignment().map(|a| a.count()).unwrap_or(0) == 0 {
        assert!(Instant::now() < deadline, "no assignment on events.raw");
        let _ = tokio::time::timeout(Duration::from_millis(200), c.recv()).await;
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
    c
}

/// Zbiera koperty o danym `schema` aż do `want` unikalnych wartości `payload.i` (albo timeout). Zwraca wszystkie (z duplikatami).
async fn collect(
    c: &StreamConsumer,
    schema: &str,
    want: usize,
    timeout: Duration,
) -> Vec<(Envelope, usize)> {
    let mut out = Vec::new();
    let mut uniq = HashSet::new();
    let deadline = tokio::time::Instant::now() + timeout;
    while uniq.len() < want {
        let Ok(res) = tokio::time::timeout_at(deadline, c.recv()).await else {
            break;
        };
        let Ok(msg) = res else { continue };
        if let Ok(env) = kafka::decode::<Envelope>(&msg) {
            if env.schema == schema {
                let i = env.payload["i"].as_u64().unwrap() as usize;
                uniq.insert(i);
                out.push((env, i));
            }
        }
    }
    out
}

async fn stop(tx: watch::Sender<bool>, h: JoinHandle<()>) {
    tx.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(30), h)
        .await
        .expect("source did not stop")
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn throughput_order_and_commit_after_effects() {
    const N: usize = 2000;
    const KEYS: usize = 50;
    let env = Env::new().await;
    env.send(0..N, KEYS).await;
    let tail = raw_tail().await;

    let start = Instant::now();
    let (tx, h) = env.start();
    let got = collect(&tail, &env.schema, N, Duration::from_secs(90)).await;
    let elapsed = start.elapsed();
    stop(tx, h).await;

    let uniq: HashSet<usize> = got.iter().map(|(_, i)| *i).collect();
    println!(
        "{N} wiadomości źródła -> events.raw w {elapsed:?} ({:.0} msg/s), unikalnych {}",
        N as f64 / elapsed.as_secs_f64(),
        uniq.len()
    );
    assert_eq!(
        uniq.len(),
        N,
        "nie wszystkie wiadomości trafiły na events.raw"
    );
    assert!(got
        .iter()
        .all(|(e, _)| e.source == format!("kafka:{}", env.topic)));
    assert!(elapsed < Duration::from_secs(30), "zbyt wolno: {elapsed:?}");

    let mut per_key: HashMap<usize, Vec<(chrono::DateTime<chrono::Utc>, usize)>> = HashMap::new();
    for (e, i) in &got {
        per_key
            .entry(i % KEYS)
            .or_default()
            .push((e.received_at, *i));
    }
    for (k, mut v) in per_key {
        v.sort();
        assert!(
            v.windows(2).all(|w| w[0].1 < w[1].1),
            "naruszona kolejność dla klucza k{k}"
        );
    }

    assert_eq!(env.committed_sum(), N as i64);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn restart_loses_nothing_and_skips_no_offsets() {
    const N: usize = 1500;
    let env = Env::new().await;
    let tail = raw_tail().await;

    let (tx, h) = env.start();
    env.send(0..N / 2, 20).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    stop(tx, h).await;
    let mid = env.committed_sum();
    assert!(mid <= (N / 2) as i64);

    let (tx, h) = env.start();
    env.send(N / 2..N, 20).await;
    let got = collect(&tail, &env.schema, N, Duration::from_secs(90)).await;
    stop(tx, h).await;

    let uniq: HashSet<usize> = got.iter().map(|(_, i)| *i).collect();
    println!("restart: wysłano {N}, na events.raw {} (z duplikatami), unikalnych {}, committed po 1. stopie {mid}", got.len(), uniq.len());
    assert_eq!(uniq.len(), N, "zgubione wiadomości po restarcie");
    assert!(got.len() >= N);
    assert_eq!(
        env.committed_sum(),
        N as i64,
        "committed != wysłane (przeskoczony offset albo niedokończone)"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bad_message_is_recorded_and_committed_without_blocking() {
    let env = Env::new().await;
    let tail = raw_tail().await;
    let (tx, h) = env.start();
    for k in 0..3 {
        env.producer
            .send(
                FutureRecord::to(&env.topic).key("k0").payload("{not json"),
                Duration::from_secs(10),
            )
            .await
            .map_err(|(e, _)| e)
            .unwrap();
        env.send(k * 10..k * 10 + 10, 1).await;
    }
    let got = collect(&tail, &env.schema, 30, Duration::from_secs(60)).await;
    let deadline = Instant::now() + Duration::from_secs(20);
    while env.error_count().await < 3 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    stop(tx, h).await;
    assert_eq!(
        got.iter().map(|(_, i)| *i).collect::<HashSet<_>>().len(),
        30
    );
    assert_eq!(env.error_count().await, 3);
    assert_eq!(env.committed_sum(), 33);
}
