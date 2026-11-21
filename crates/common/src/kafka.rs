//! Kafka: producent i konsument wiadomości JSON.
//!
//! Zasada (design D2): konsument zatwierdza offset DOPIERO po zakończeniu pracy
//! i zapisaniu jej skutków (`commit`). Dlatego `enable.auto.commit=false`.

use std::time::Duration;

use anyhow::Context;
use rdkafka::config::ClientConfig;
use rdkafka::consumer::{CommitMode, Consumer, StreamConsumer};
use rdkafka::message::{BorrowedMessage, Message};
use rdkafka::producer::{FutureProducer, FutureRecord};
use serde::{de::DeserializeOwned, Serialize};

pub use rdkafka;

/// Producent o niskiej latencji (ścieżka alertu). `acks=all` – brak utraty po 202.
pub fn producer(brokers: &str) -> anyhow::Result<FutureProducer> {
    ClientConfig::new()
        .set("bootstrap.servers", brokers)
        .set("acks", "all")
        .set("enable.idempotence", "true")
        .set("linger.ms", "1")
        .set("compression.type", "lz4")
        .set("message.timeout.ms", "10000")
        .create()
        .context("creating kafka producer")
}

/// Wysyła wiadomość jako JSON i czeka na potwierdzenie brokera.
pub async fn publish<T: Serialize>(
    producer: &FutureProducer,
    topic: &str,
    key: &str,
    message: &T,
) -> anyhow::Result<()> {
    let payload = serde_json::to_vec(message).context("serializing kafka message")?;
    producer
        .send(
            FutureRecord::to(topic).key(key).payload(&payload),
            Duration::from_secs(10),
        )
        .await
        .map_err(|(e, _)| anyhow::anyhow!("publishing to {topic}: {e}"))?;
    Ok(())
}

/// Konsument w grupie `group`, bez auto-commitu, czytający od początku przy pierwszym starcie.
pub fn consumer(brokers: &str, group: &str, topics: &[&str]) -> anyhow::Result<StreamConsumer> {
    let consumer: StreamConsumer = ClientConfig::new()
        .set("bootstrap.servers", brokers)
        .set("group.id", group)
        .set("enable.auto.commit", "false")
        .set("auto.offset.reset", "earliest")
        .set("session.timeout.ms", "10000")
        .create()
        .context("creating kafka consumer")?;
    consumer.subscribe(topics).context("subscribing")?;
    Ok(consumer)
}

/// Dekoduje payload wiadomości z JSON.
pub fn decode<T: DeserializeOwned>(message: &BorrowedMessage<'_>) -> anyhow::Result<T> {
    let payload = message.payload().context("empty kafka message")?;
    serde_json::from_slice(payload)
        .with_context(|| format!("decoding message from {}", message.topic()))
}

/// Zatwierdza offset przetworzonej wiadomości.
pub fn commit(consumer: &StreamConsumer, message: &BorrowedMessage<'_>) -> anyhow::Result<()> {
    consumer
        .commit_message(message, CommitMode::Async)
        .context("committing offset")
}
