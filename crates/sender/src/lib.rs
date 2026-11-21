//! `sender` – dostarczanie powiadomień przez adaptery kanałów, retry z backoffem,
//! przełączanie po łańcuchu fallbacku oraz DLQ (właściciel: P2, sekcje 9.1–9.3).

pub mod adapters;
pub mod engine;
pub mod store;

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use common::kafka::rdkafka::consumer::{CommitMode, Consumer};
use common::kafka::rdkafka::{Message, Offset, TopicPartitionList};
use contracts::{topics, DeliveryRequest};
use tokio::sync::{mpsc, Semaphore};

/// Domyślna współbieżność przetwarzania `deliveries.requested` (konfigurowalna przez `SENDER_CONCURRENCY`).
pub const DEFAULT_SENDER_CONCURRENCY: usize = 32;

pub use common::kafka::OffsetTracker;

fn sender_concurrency() -> usize {
    std::env::var("SENDER_CONCURRENCY")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&v| v >= 1)
        .unwrap_or(DEFAULT_SENDER_CONCURRENCY)
}

fn commit_partition_offset(
    consumer: &common::kafka::rdkafka::consumer::StreamConsumer,
    topic: &str,
    partition: i32,
    offset: i64,
) {
    let mut tpl = TopicPartitionList::new();
    if let Err(e) = tpl.add_partition_offset(topic, partition, Offset::Offset(offset + 1)) {
        tracing::warn!(error = %e, topic, partition, offset, "failed to build TopicPartitionList");
        return;
    }
    if let Err(e) = consumer.commit(&tpl, CommitMode::Async) {
        tracing::warn!(error = %e, topic, partition, offset, "failed to commit offset");
    }
}

pub use adapters::{
    format_email_body, format_phone_body, format_sms_body, ChannelAdapter, EmailAdapter, PhoneMockAdapter, SendError,
    SmsMockAdapter, UiAdapter,
};
pub use engine::{ProcessOutcome, SenderConfig, SenderEngine, DEFAULT_BACKOFF_MS, DEFAULT_MAX_ATTEMPTS};
pub use store::{
    channel_to_str, db, status_to_str, str_to_channel, str_to_status, ClaimOutcome, DeliveryBackend, DeliveryRecord,
    InMemoryBackend, MockChannelState, MockMessage, PgKafkaBackend, UpsertDelivery,
};

/// Sygnał zatrzymania dla pętli konsumentów (`true` = zatrzymaj).
pub type Shutdown = tokio::sync::watch::Receiver<bool>;

/// Rola `sender`: konsumuje `deliveries.requested`, przetwarza zlecenia współbieżnie
/// (limit `SENDER_CONCURRENCY`, domyślnie 32) z commitem offsetów ściśle w kolejności per partycja
/// po zapisie skutków w Postgresie / Kafce (design D2, D8, D9, hardening 2.4).
pub async fn run_sender(brokers: String, engine: SenderEngine, mut shutdown: Shutdown) -> anyhow::Result<()> {
    let consumer = common::kafka::consumer(&brokers, topics::groups::SENDER, &[topics::DELIVERIES_REQUESTED])?;
    let concurrency = sender_concurrency();
    tracing::info!(
        topic = topics::DELIVERIES_REQUESTED,
        group = topics::groups::SENDER,
        concurrency,
        "sender consumer started"
    );

    let sem = Arc::new(Semaphore::new(concurrency));
    let (done_tx, mut done_rx) = mpsc::unbounded_channel::<(i32, i64, bool)>();
    let mut tracker = OffsetTracker::default();

    loop {
        tokio::select! {
            biased;
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    tracing::info!(in_flight = tracker.in_flight(), "sender consumer shutting down, draining in-flight tasks");
                    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
                    while tracker.in_flight() > 0 {
                        match tokio::time::timeout_at(deadline, done_rx.recv()).await {
                            Ok(Some((partition, offset, true))) => {
                                if let Some(off) = tracker.mark_done(partition, offset) {
                                    commit_partition_offset(&consumer, topics::DELIVERIES_REQUESTED, partition, off);
                                }
                            }
                            Ok(Some(_)) => {}
                            Ok(None) | Err(_) => break,
                        }
                    }
                    return Ok(());
                }
            }
            Some((partition, offset, ok)) = done_rx.recv() => {
                if ok {
                    if let Some(off) = tracker.mark_done(partition, offset) {
                        commit_partition_offset(&consumer, topics::DELIVERIES_REQUESTED, partition, off);
                    }
                }
            }
            permit = sem.clone().acquire_owned() => {
                let permit = match permit {
                    Ok(p) => p,
                    Err(_) => return Ok(()),
                };
                tokio::select! {
                    _ = shutdown.changed() => {
                        if *shutdown.borrow() {
                            drop(permit);
                            continue;
                        }
                    }
                    Some((partition, offset, ok)) = done_rx.recv() => {
                        drop(permit);
                        if ok {
                            if let Some(off) = tracker.mark_done(partition, offset) {
                                commit_partition_offset(&consumer, topics::DELIVERIES_REQUESTED, partition, off);
                            }
                        }
                    }
                    msg_res = consumer.recv() => {
                        let msg = match msg_res {
                            Ok(m) => m,
                            Err(e) => {
                                drop(permit);
                                tracing::warn!(error = %e, "kafka recv error on deliveries.requested");
                                tokio::time::sleep(Duration::from_millis(200)).await;
                                continue;
                            }
                        };

                        let partition = msg.partition();
                        let offset = msg.offset();
                        tracker.track(partition, offset);

                        let req: DeliveryRequest = match common::kafka::decode(&msg) {
                            Ok(r) => r,
                            Err(e) => {
                                drop(permit);
                                tracing::error!(error = %e, partition, offset, "invalid DeliveryRequest on deliveries.requested; committing poison message");
                                if let Some(off) = tracker.mark_done(partition, offset) {
                                    commit_partition_offset(&consumer, topics::DELIVERIES_REQUESTED, partition, off);
                                }
                                continue;
                            }
                        };

                        let eng = engine.clone();
                        let tx = done_tx.clone();
                        tokio::spawn(async move {
                            let _permit = permit;
                            let ok = match eng.process_request(req).await {
                                Ok(outcome) => {
                                    tracing::debug!(?outcome, partition, offset, "processed delivery request");
                                    true
                                }
                                Err(e) => {
                                    tracing::error!(error = %e, partition, offset, "failed to process delivery request; leaving offset uncommitted");
                                    false
                                }
                            };
                            let _ = tx.send((partition, offset, ok));
                        });
                    }
                }
            }
        }
    }
}

/// Rola `sender-retry`: konsumuje `deliveries.retry`, czeka do `not_before` (z możliwością
/// przerwania przy shutdownie bez commitu offsetu) i ponawia dostarczenie przez `SenderEngine`.
pub async fn run_sender_retry(brokers: String, engine: SenderEngine, mut shutdown: Shutdown) -> anyhow::Result<()> {
    let consumer = common::kafka::consumer(&brokers, topics::groups::SENDER_RETRY, &[topics::DELIVERIES_RETRY])?;
    tracing::info!(
        topic = topics::DELIVERIES_RETRY,
        group = topics::groups::SENDER_RETRY,
        "sender-retry consumer started"
    );

    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    tracing::info!("sender-retry consumer shutting down");
                    return Ok(());
                }
            }
            msg_res = consumer.recv() => {
                let msg = match msg_res {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::warn!(error = %e, "kafka recv error on deliveries.retry");
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        continue;
                    }
                };

                let req: DeliveryRequest = match common::kafka::decode(&msg) {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::error!(error = %e, "invalid DeliveryRequest on deliveries.retry; committing poison message");
                        let _ = common::kafka::commit(&consumer, &msg);
                        continue;
                    }
                };

                if let Some(not_before) = req.not_before {
                    let now = Utc::now();
                    if not_before > now {
                        let wait = (not_before - now).to_std().unwrap_or(Duration::ZERO);
                        tokio::select! {
                            _ = tokio::time::sleep(wait) => {}
                            _ = shutdown.changed() => {
                                tracing::info!("shutdown while waiting for retry not_before; leaving offset uncommitted");
                                return Ok(());
                            }
                        }
                    }
                }

                match engine.process_request(req).await {
                    Ok(outcome) => {
                        tracing::debug!(?outcome, "processed retry delivery request");
                        if let Err(e) = common::kafka::commit(&consumer, &msg) {
                            tracing::warn!(error = %e, "failed to commit offset on deliveries.retry");
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "failed to process retry delivery request; leaving offset uncommitted");
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                }
            }
        }
    }
}
