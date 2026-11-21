//! loadgen – generator ruchu HTTP + Kafka do testu obciążenia (właściciel: P1).
//!
//! Konfiguracja i logika: `lib.rs`. Tutaj: pętla tempa i wysyłki.
//! Uruchomienie i interpretacja wyników: `docs/LOADTEST.md`.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use futures::{stream, StreamExt};
use loadgen::{channel_for, Channel, Event, Generator, Mode, Settings};
use common::kafka::rdkafka::producer::FutureProducer;
use tokio::time::MissedTickBehavior;

/// Krok planowania tempa: co 10 ms dolewamy `RATE * elapsed` zdarzeń.
const TICK: Duration = Duration::from_millis(10);
/// Maksymalna liczba równoczesnych wysyłek.
const CONCURRENCY: usize = 64;
/// Ile pierwszych błędów wypisać na stderr (reszta tylko w liczniku).
const MAX_LOGGED_ERRORS: u64 = 5;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    common::init_tracing("loadgen");
    let settings = Settings::from_env()?;
    let config = common::Config::from_env();

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("creating http client")?;
    let producer: Option<FutureProducer> = match settings.mode {
        Mode::Http => None,
        Mode::Kafka | Mode::Both => Some(common::kafka::producer(&config.kafka_brokers)?),
    };
    let ingest_url = format!("{}/ingest", settings.ingest_url.trim_end_matches('/'));

    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
    // Równoległe instancje (LOADGEN_ID=0,1,..) dostają rozłączne zakresy sufiksów i inne ziarno,
    // żeby ich „unikalne” zdarzenia nie zlewały się w dedupie.
    let instance: u64 = std::env::var("LOADGEN_ID").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    let seed = now_ms ^ 0x9E37_79B9_7F4A_7C15 ^ instance.wrapping_mul(0xD6E8_FEB8_6659_FD93);
    let mut generator = Generator::new(settings.dup_ratio, seed, now_ms + instance * 1_000_000_000_000);

    println!(
        "loadgen: tryb={:?}, tempo={} msg/s, czas={} s, duplikaty={:.2}, ingest={ingest_url}, temat={}",
        settings.mode,
        settings.rate,
        settings.duration.as_secs(),
        settings.dup_ratio,
        settings.source_topic,
    );

    let mut ticker = tokio::time::interval(TICK);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let started = Instant::now();
    let mut scheduled: u64 = 0;
    let mut stats = Stats::default();

    loop {
        ticker.tick().await;
        let elapsed = started.elapsed();
        if elapsed >= settings.duration {
            break;
        }
        // Liczba zdarzeń, które powinny być już wysłane wg zadanego tempa.
        let due = (elapsed.as_secs_f64() * settings.rate as f64) as u64;
        let batch: Vec<Event> = (scheduled..due)
            .map(|seq| generator.next_event(channel_for(settings.mode, seq)))
            .collect();
        scheduled = due;

        let http = &http;
        let producer = producer.as_ref();
        let ingest_url = ingest_url.as_str();
        let topic = settings.source_topic.as_str();
        stream::iter(batch)
            .map(|event| send(http, ingest_url, producer, topic, event))
            .buffer_unordered(CONCURRENCY)
            .for_each(|result| {
                stats.record(result);
                futures::future::ready(())
            })
            .await;
    }

    let elapsed = started.elapsed().as_secs_f64().max(f64::EPSILON);
    println!("wysłane: {}", stats.sent);
    println!("błędy: {}", stats.errors);
    println!("zaplanowane: {}", scheduled);
    println!("osiągnięte msg/s: {:.1} (czas {:.2} s)", stats.sent as f64 / elapsed, elapsed);
    println!(
        "wygenerowane: {}, duplikaty: {} ({:.1}%), unikalne fingerprinty: {}",
        generator.generated(),
        generator.duplicates(),
        100.0 * generator.duplicates() as f64 / generator.generated().max(1) as f64,
        generator.unique_identities(),
    );
    Ok(())
}

/// Wysyła jedno zdarzenie na właściwy kanał.
async fn send(
    http: &reqwest::Client,
    ingest_url: &str,
    producer: Option<&FutureProducer>,
    topic: &str,
    event: Event,
) -> anyhow::Result<()> {
    match event.channel {
        Channel::Http => {
            http.post(ingest_url).json(&event.body).send().await?.error_for_status()?;
            Ok(())
        }
        Channel::Kafka => {
            let producer = producer.context("producer Kafka niedostępny")?;
            common::kafka::publish(producer, topic, event.schema.name(), &event.body).await
        }
    }
}

#[derive(Default)]
struct Stats {
    sent: u64,
    errors: u64,
}

impl Stats {
    fn record(&mut self, result: anyhow::Result<()>) {
        match result {
            Ok(()) => self.sent += 1,
            Err(e) => {
                self.errors += 1;
                if self.errors <= MAX_LOGGED_ERRORS {
                    tracing::warn!(error = %e, "wysyłka nieudana");
                }
            }
        }
    }
}
