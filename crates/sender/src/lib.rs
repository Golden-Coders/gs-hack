//! `sender` – dostarczanie powiadomień przez adaptery kanałów, retry z backoffem,
//! przełączanie po łańcuchu fallbacku oraz DLQ (właściciel: P2, sekcje 9.1–9.3).

pub mod adapters;
pub mod engine;
pub mod store;

use std::time::Duration;

use chrono::Utc;
use contracts::{topics, DeliveryRequest};

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

/// Rola `sender`: konsumuje `deliveries.requested`, wykonuje dostarczenie przez `SenderEngine`
/// i zatwierdza offset dopiero po zapisie skutków w Postgresie / Kafce (design D2, D8, D9).
pub async fn run_sender(brokers: String, engine: SenderEngine, mut shutdown: Shutdown) -> anyhow::Result<()> {
    let consumer = common::kafka::consumer(&brokers, topics::groups::SENDER, &[topics::DELIVERIES_REQUESTED])?;
    tracing::info!(topic = topics::DELIVERIES_REQUESTED, group = topics::groups::SENDER, "sender consumer started");

    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    tracing::info!("sender consumer shutting down");
                    return Ok(());
                }
            }
            msg_res = consumer.recv() => {
                let msg = match msg_res {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::warn!(error = %e, "kafka recv error on deliveries.requested");
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        continue;
                    }
                };

                let req: DeliveryRequest = match common::kafka::decode(&msg) {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::error!(error = %e, "invalid DeliveryRequest on deliveries.requested; committing poison message");
                        let _ = common::kafka::commit(&consumer, &msg);
                        continue;
                    }
                };

                match engine.process_request(req).await {
                    Ok(outcome) => {
                        tracing::debug!(?outcome, "processed delivery request");
                        if let Err(e) = common::kafka::commit(&consumer, &msg) {
                            tracing::warn!(error = %e, "failed to commit offset on deliveries.requested");
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "failed to process delivery request; will retry before offset commit");
                        tokio::time::sleep(Duration::from_millis(500)).await;
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
