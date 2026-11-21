//! Normalizer: `events.raw` -> `events.normalized` (albo UNMAPPED) + replay UNMAPPED po zmianie mappingów.

use std::time::Duration;

use common::db::sqlx::{PgPool, Row};
use common::kafka::rdkafka::consumer::CommitMode;
use common::kafka::rdkafka::consumer::Consumer;
use common::kafka::rdkafka::message::BorrowedMessage;
use common::kafka::rdkafka::producer::FutureProducer;
use contracts::{topics, Envelope, UiEvent, UiEventKind};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::config::MappingStore;
use crate::mapping::parse_payload;
use crate::{Shutdown, Topics};

const MAX_REPLAY_ROWS_PER_TRIGGER: usize = 5_000;
const REPLAY_BATCH_SIZE: usize = 500;

/// Konsument `events.raw` w grupie `lifecycle-normalizer`; działa do sygnału zatrzymania.
pub async fn run_normalizer(
    brokers: String,
    pool: PgPool,
    producer: FutureProducer,
    store: MappingStore,
    shutdown: Shutdown,
) -> anyhow::Result<()> {
    run_normalizer_with(topics::groups::LIFECYCLE_NORMALIZER, Topics::default(), brokers, pool, producer, store, shutdown).await
}

/// Jak `run_normalizer`, ale z podaną grupą i topikami (testy używają własnych, izolowanych).
pub async fn run_normalizer_with(
    group: &str,
    topics: Topics,
    brokers: String,
    pool: PgPool,
    producer: FutureProducer,
    store: MappingStore,
    mut shutdown: Shutdown,
) -> anyhow::Result<()> {
    let consumer = common::kafka::consumer(&brokers, group, &[topics.events_raw.as_str()])?;
    loop {
        let msg = tokio::select! {
            _ = shutdown.changed() => return Ok(()),
            m = consumer.recv() => match m {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(error = %e, "normalizer: kafka error");
                    tokio::select! {
                        _ = shutdown.changed() => return Ok(()),
                        _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                    }
                    continue;
                }
            },
        };
        // At-least-once: ponawiamy do skutku (z limitem dla trwałych błędów danych/indeksu Postgresa),
        // offset commitujemy dopiero po zapisie skutków.
        let mut permanent_failures = 0u32;
        loop {
            match handle(&msg, &pool, &producer, &store, &topics).await {
                Ok(()) => break,
                Err(e) => {
                    if crate::is_permanent_pg_error(&e) {
                        permanent_failures += 1;
                        if permanent_failures >= crate::MAX_PERMANENT_RETRIES {
                            tracing::error!(error = %e, retries = permanent_failures, "normalizer: permanent postgres error, skipping message");
                            break;
                        }
                    }
                    tracing::warn!(error = %e, "normalizer: processing failed, retrying");
                    tokio::select! {
                        _ = shutdown.changed() => return Ok(()),
                        _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                    }
                }
            }
        }
        if let Err(e) = consumer.commit_message(&msg, CommitMode::Async) {
            tracing::warn!(error = %e, "normalizer: commit failed");
        }
    }
}

async fn handle(
    msg: &BorrowedMessage<'_>,
    pool: &PgPool,
    producer: &FutureProducer,
    store: &MappingStore,
    topics: &Topics,
) -> anyhow::Result<()> {
    let env: Envelope = match common::kafka::decode(msg) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(error = %e, "normalizer: undecodable envelope, skipped");
            return Ok(());
        }
    };
    let raw = match parse_payload(&env.format, &env.payload) {
        Ok(v) => v,
        Err(e) => {
            // Ingest już waliduje format; tu tylko zabezpieczenie.
            tracing::warn!(event_id = %env.event_id, error = %e, "normalizer: payload parse failed, skipped");
            return Ok(());
        }
    };
    let mapping = store.load().get(&env.schema).cloned();
    match mapping {
        Some(m) => {
            let event = m.normalize(&env, raw);
            common::kafka::publish(producer, &topics.events_normalized, &event.fingerprint, &event).await?;
        }
        None => {
            let mut schema = env.schema.clone();
            crate::mapping::strip_nuls_str(&mut schema);
            let mut env_json = serde_json::to_value(&env)?;
            crate::mapping::strip_nuls_value(&mut env_json);
            common::db::sqlx::query(
                "INSERT INTO unmapped_events (event_id, schema, envelope, received_at) VALUES ($1, $2, $3, $4)
                 ON CONFLICT (event_id) DO NOTHING",
            )
            .bind(env.event_id)
            .bind(&schema)
            .bind(&env_json)
            .bind(env.received_at)
            .execute(pool)
            .await?;
            let count: i64 = common::db::sqlx::query("SELECT count(*) AS n FROM unmapped_events WHERE schema = $1")
                .bind(&schema)
                .fetch_one(pool)
                .await?
                .get("n");
            common::kafka::publish(producer, &topics.events_unmapped, &schema, &env).await?;
            let ui = UiEvent {
                alert_id: None,
                kind: UiEventKind::Unmapped,
                payload: json!({"schema": schema, "count": count}),
                at: chrono::Utc::now(),
            };
            common::kafka::publish(producer, &topics.ui_events, "unmapped", &ui).await?;
        }
    }
    Ok(())
}

/// Ponowne przetworzenie UNMAPPED dla schematów, które mają już mapping: koperty wracają na `events.raw`.
/// Transakcja z `FOR UPDATE SKIP LOCKED` – bezpieczne przy wielu instancjach; awaria przed commitem
/// transakcji = ponowna publikacja (at-least-once; deduplikacja po `event_id` nie jest potrzebna,
/// bo powtórzone zdarzenie co najwyżej zwiększy licznik).
pub async fn replay_unmapped(pool: &PgPool, producer: &FutureProducer, store: &MappingStore) -> anyhow::Result<()> {
    replay_unmapped_to(pool, producer, store, topics::EVENTS_RAW).await
}

/// Jak `replay_unmapped`, ale na podany topik wejściowy (testy).
pub async fn replay_unmapped_to(pool: &PgPool, producer: &FutureProducer, store: &MappingStore, events_raw: &str) -> anyhow::Result<()> {
    let schemas: Vec<String> = common::db::sqlx::query("SELECT DISTINCT schema FROM unmapped_events")
        .fetch_all(pool)
        .await?
        .iter()
        .map(|r| r.get("schema"))
        .collect();
    let mapped = store.load();
    let mut remaining = MAX_REPLAY_ROWS_PER_TRIGGER;
    for schema in schemas.into_iter().filter(|s| mapped.contains_key(s)) {
        if remaining == 0 {
            tracing::warn!(limit = MAX_REPLAY_ROWS_PER_TRIGGER, "replay_unmapped reached per-trigger row cap, deferring remaining rows");
            break;
        }
        let mut total = 0usize;
        while remaining > 0 {
            let batch_limit = remaining.min(REPLAY_BATCH_SIZE) as i64;
            let mut tx = pool.begin().await?;
            let rows = common::db::sqlx::query(
                "SELECT event_id, envelope FROM unmapped_events WHERE schema = $1
                 ORDER BY received_at LIMIT $2 FOR UPDATE SKIP LOCKED",
            )
            .bind(&schema)
            .bind(batch_limit)
            .fetch_all(&mut *tx)
            .await?;
            if rows.is_empty() {
                break;
            }
            let mut ids: Vec<Uuid> = Vec::with_capacity(rows.len());
            for row in &rows {
                let id: Uuid = row.get("event_id");
                let envelope: Value = row.get("envelope");
                common::kafka::publish(producer, events_raw, &id.to_string(), &envelope).await?;
                ids.push(id);
            }
            common::db::sqlx::query("DELETE FROM unmapped_events WHERE event_id = ANY($1)")
                .bind(&ids)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            total += ids.len();
            remaining = remaining.saturating_sub(ids.len());
        }
        if total > 0 {
            tracing::info!(%schema, total, "replayed unmapped events");
        }
    }
    Ok(())
}
