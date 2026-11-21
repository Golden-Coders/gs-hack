//! Test integracyjny `run_concurrent` z prawdziwym brokerem (pomijany, gdy broker niedostępny).

use std::collections::HashMap;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::kafka::rdkafka::admin::{AdminClient, AdminOptions, NewTopic, TopicReplication};
use common::kafka::rdkafka::client::DefaultClientContext;
use common::kafka::rdkafka::config::ClientConfig;
use common::kafka::rdkafka::consumer::{Consumer, StreamConsumer};
use common::kafka::rdkafka::message::{Message, OwnedMessage};
use common::kafka::rdkafka::{Offset, TopicPartitionList};
use common::kafka::{concurrent_consumer, run_concurrent, ConcurrentConfig, LaneHandler};

#[derive(Clone)]
struct Collect(Arc<Mutex<Vec<(String, i64)>>>);

impl LaneHandler for Collect {
    async fn handle(&mut self, msg: &OwnedMessage) -> anyhow::Result<()> {
        let key = String::from_utf8_lossy(msg.key().unwrap_or_default()).into_owned();
        self.0.lock().unwrap().push((key, msg.offset()));
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn consumes_all_in_key_order_and_commits_on_shutdown() {
    let brokers = std::env::var("KAFKA_BROKERS").unwrap_or_else(|_| "localhost:19092".into());
    let reachable = brokers
        .to_socket_addrs()
        .ok()
        .and_then(|mut a| a.next())
        .is_some_and(|a| TcpStream::connect_timeout(&a, Duration::from_millis(500)).is_ok());
    if !reachable {
        eprintln!("skipping: kafka broker {brokers} unreachable");
        return;
    }
    let id = format!("{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
    let topic = format!("conc-test-{id}");
    let group = format!("conc-group-{id}");
    let admin: AdminClient<DefaultClientContext> =
        ClientConfig::new().set("bootstrap.servers", &brokers).create().unwrap();
    admin
        .create_topics(&[NewTopic::new(&topic, 2, TopicReplication::Fixed(1))], &AdminOptions::new())
        .await
        .unwrap();
    let producer = common::kafka::producer(&brokers).unwrap();
    for i in 0..200 {
        common::kafka::publish(&producer, &topic, &format!("k{}", i % 10), &i).await.unwrap();
    }

    let seen = Arc::new(Mutex::new(Vec::new()));
    let consumer = concurrent_consumer(&brokers, &group, &[&topic]).unwrap();
    let (tx, rx) = tokio::sync::watch::channel(false);
    let handler = Collect(seen.clone());
    let cfg = ConcurrentConfig::from_env("CONC_IT", 4);
    let task = tokio::spawn(run_concurrent(consumer, cfg, rx, move |_| handler.clone()));
    let start = Instant::now();
    while seen.lock().unwrap().len() < 200 {
        assert!(start.elapsed() < Duration::from_secs(60), "timeout");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tx.send(true).unwrap();
    task.await.unwrap().unwrap();

    let mut per_key: HashMap<String, Vec<i64>> = HashMap::new();
    for (k, o) in seen.lock().unwrap().iter() {
        per_key.entry(k.clone()).or_default().push(*o);
    }
    for (k, offs) in per_key {
        assert!(offs.windows(2).all(|w| w[0] < w[1]), "order violated for {k}");
    }

    let check: StreamConsumer = ClientConfig::new()
        .set("bootstrap.servers", &brokers)
        .set("group.id", &group)
        .set("enable.auto.commit", "false")
        .create()
        .unwrap();
    let mut tpl = TopicPartitionList::new();
    for p in 0..2 {
        tpl.add_partition_offset(&topic, p, Offset::Invalid).unwrap();
    }
    let committed = check.committed_offsets(tpl, Duration::from_secs(10)).unwrap();
    let total: i64 = committed
        .elements()
        .iter()
        .map(|e| match e.offset() {
            Offset::Offset(o) => o,
            _ => 0,
        })
        .sum();
    assert_eq!(total, 200, "all offsets committed after shutdown");
}
