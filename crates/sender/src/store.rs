//! Warstwa trwałości i publikacji zdarzeń dla `sender` (Postgres + Kafka oraz wariant in-memory do testów).

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use anyhow::Context;
use chrono::Utc;
use contracts::{topics, Channel, DeliveryRequest, DeliveryStatus, Timestamp, UiEvent};
use rdkafka::producer::FutureProducer;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use uuid::Uuid;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Wiersz tabeli `deliveries` zgodny ze schematem `Delivery` w `contracts/openapi.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryRecord {
    pub id: Uuid,
    pub alert_id: Uuid,
    pub episode: i32,
    pub person_id: String,
    pub channel: Channel,
    pub status: DeliveryStatus,
    pub attempts: i32,
    pub last_error: Option<String>,
    pub received_at: Timestamp,
    pub delivered_at: Option<Timestamp>,
    pub updated_at: Timestamp,
}

/// Wiersz tabeli `mock_outbox` zgodny ze schematem `MockMessage` w `contracts/openapi.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MockMessage {
    pub id: i64,
    pub channel: Channel,
    pub to: String,
    pub person_id: String,
    pub title: String,
    pub body: String,
    pub alert_id: Option<Uuid>,
    pub sent_at: Timestamp,
}

/// Stan dostępności kanału w `mock_channels` (`MockChannelState` w `contracts/openapi.yaml`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MockChannelState {
    pub channel: Channel,
    pub available: bool,
}

pub fn channel_to_str(channel: Channel) -> &'static str {
    match channel {
        Channel::Ui => "ui",
        Channel::Email => "email",
        Channel::Sms => "sms",
        Channel::Phone => "phone",
    }
}

pub fn str_to_channel(s: &str) -> anyhow::Result<Channel> {
    match s {
        "ui" => Ok(Channel::Ui),
        "email" => Ok(Channel::Email),
        "sms" => Ok(Channel::Sms),
        "phone" => Ok(Channel::Phone),
        other => anyhow::bail!("unknown channel: {other}"),
    }
}

pub fn status_to_str(status: DeliveryStatus) -> &'static str {
    match status {
        DeliveryStatus::Pending => "pending",
        DeliveryStatus::Retrying => "retrying",
        DeliveryStatus::Delivered => "delivered",
        DeliveryStatus::Fallback => "fallback",
        DeliveryStatus::Failed => "failed",
        DeliveryStatus::Dlq => "dlq",
    }
}

pub fn str_to_status(s: &str) -> anyhow::Result<DeliveryStatus> {
    match s {
        "pending" => Ok(DeliveryStatus::Pending),
        "retrying" => Ok(DeliveryStatus::Retrying),
        "delivered" => Ok(DeliveryStatus::Delivered),
        "fallback" => Ok(DeliveryStatus::Fallback),
        "failed" => Ok(DeliveryStatus::Failed),
        "dlq" => Ok(DeliveryStatus::Dlq),
        other => anyhow::bail!("unknown delivery status: {other}"),
    }
}

/// Parametry zapisu stanu dostarczenia w tabeli `deliveries`.
#[derive(Debug, Clone)]
pub struct UpsertDelivery<'a> {
    pub idempotency_key: &'a str,
    pub request: &'a DeliveryRequest,
    pub channel: Channel,
    pub status: DeliveryStatus,
    pub attempts: i32,
    pub last_error: Option<&'a str>,
    pub delivered_at: Option<Timestamp>,
}

/// Wynik atomowego przejęcia dostarczenia do wysyłki (eliminuje wyścig `sender`/`sender-retry` na tym samym kluczu).
#[derive(Debug, Clone, PartialEq)]
pub enum ClaimOutcome {
    /// Dostarczenie przejęte przez bieżącego workera (`status = 'pending'`, `attempts = attempt_num`).
    Claimed(DeliveryRecord),
    /// Dostarczenie zostało już wcześniej zakończone sukcesem (`status = 'delivered'`).
    AlreadyDelivered(DeliveryRecord),
    /// Inny worker właśnie wysyła to dostarczenie (`status = 'pending'`, `attempts > 0`, świeży lease < 60 s).
    InProgress(DeliveryRecord),
}

/// Czas ważności lease'a `pending` przed uznaniem, że poprzedni worker uległ awarii w trakcie wysyłki.
const PENDING_LEASE_SECS: i64 = 60;

/// Abstrakcja nad bazą danych (`deliveries`, `mock_outbox`, `mock_channels`) oraz publikacją na topiki Kafki.
pub trait DeliveryBackend: Send + Sync {
    /// Sprawdza, czy dany kanał jest włączony w `mock_channels` (domyślnie `true`).
    fn is_channel_available(&self, channel: Channel) -> BoxFuture<'_, anyhow::Result<bool>>;

    /// Pobiera dostarczenie po kluczu idempotencji (`delivery:{alert_id}:{episode}:{person_id}:{channel}`).
    fn get_delivery_by_key(&self, idempotency_key: &str) -> BoxFuture<'_, anyhow::Result<Option<DeliveryRecord>>>;

    /// Pobiera dostarczenie wraz z zapisanym `DeliveryRequest` po identyfikatorze `id` (do `resend`).
    fn get_delivery_with_request(
        &self,
        delivery_id: Uuid,
    ) -> BoxFuture<'_, anyhow::Result<Option<(DeliveryRecord, DeliveryRequest)>>>;

    /// Atomowo przejmuje klucz idempotencji do wysyłki (chroni przed podwójną wysyłką przy wyścigu `sender`/`sender-retry`).
    fn claim_delivery<'a>(&'a self, params: UpsertDelivery<'a>) -> BoxFuture<'a, anyhow::Result<ClaimOutcome>>;

    /// Tworzy lub aktualizuje wiersz w `deliveries` (z zachowaniem istniejącego `id` przy konflikcie klucza idempotencji).
    fn upsert_delivery<'a>(&'a self, params: UpsertDelivery<'a>) -> BoxFuture<'a, anyhow::Result<DeliveryRecord>>;

    /// Zapisuje wiadomość w `mock_outbox` (dla symulowanych kanałów SMS / telefon).
    fn insert_mock_outbox<'a>(
        &'a self,
        channel: Channel,
        to: &'a str,
        person_id: &'a str,
        title: &'a str,
        body: &'a str,
        alert_id: Option<Uuid>,
    ) -> BoxFuture<'a, anyhow::Result<MockMessage>>;

    /// Publikuje zlecenie na topik `deliveries.retry` (klucz = `alert_id`).
    fn publish_retry<'a>(&'a self, req: &'a DeliveryRequest) -> BoxFuture<'a, anyhow::Result<()>>;

    /// Publikuje zlecenie na topik `deliveries.dlq` (klucz = `alert_id`).
    fn publish_dlq<'a>(&'a self, req: &'a DeliveryRequest) -> BoxFuture<'a, anyhow::Result<()>>;

    /// Publikuje zlecenie na topik `deliveries.requested` (np. przy `POST /deliveries/{id}/resend`).
    fn publish_requested<'a>(&'a self, req: &'a DeliveryRequest) -> BoxFuture<'a, anyhow::Result<()>>;

    /// Publikuje zdarzenie na topik `ui.events` (klucz = `alert_id`).
    fn publish_ui_event<'a>(&'a self, event: &'a UiEvent) -> BoxFuture<'a, anyhow::Result<()>>;
}

// ============================================================================
// Implementacja Postgres + Kafka (produkcyjna)
// ============================================================================

#[derive(Clone)]
pub struct PgKafkaBackend {
    pool: PgPool,
    producer: Option<FutureProducer>,
}

impl PgKafkaBackend {
    pub fn new(pool: PgPool, producer: FutureProducer) -> Self {
        Self { pool, producer: Some(producer) }
    }

    /// Wariant bez producenta Kafki (np. do testów integracyjnych z samym Postgresem).
    pub fn with_pool_only(pool: PgPool) -> Self {
        Self { pool, producer: None }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub fn producer(&self) -> Option<&FutureProducer> {
        self.producer.as_ref()
    }
}

fn row_to_delivery(row: &sqlx::postgres::PgRow) -> anyhow::Result<DeliveryRecord> {
    let channel_s: String = row.try_get("channel")?;
    let status_s: String = row.try_get("status")?;
    Ok(DeliveryRecord {
        id: row.try_get("id")?,
        alert_id: row.try_get("alert_id")?,
        episode: row.try_get("episode")?,
        person_id: row.try_get("person_id")?,
        channel: str_to_channel(&channel_s)?,
        status: str_to_status(&status_s)?,
        attempts: row.try_get("attempts")?,
        last_error: row.try_get("last_error")?,
        received_at: row.try_get("received_at")?,
        delivered_at: row.try_get("delivered_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

pub mod db {
    use super::*;

    pub async fn is_channel_available(pool: &PgPool, channel: Channel) -> anyhow::Result<bool> {
        let ch = channel_to_str(channel);
        let available: Option<bool> =
            sqlx::query_scalar("SELECT available FROM mock_channels WHERE channel = $1")
                .bind(ch)
                .fetch_optional(pool)
                .await
                .context("querying mock_channels")?;
        Ok(available.unwrap_or(true))
    }

    pub async fn set_channel_available(
        pool: &PgPool,
        channel: Channel,
        available: bool,
    ) -> anyhow::Result<MockChannelState> {
        let ch = channel_to_str(channel);
        sqlx::query(
            "INSERT INTO mock_channels (channel, available, updated_at)
             VALUES ($1, $2, now())
             ON CONFLICT (channel) DO UPDATE SET available = EXCLUDED.available, updated_at = now()",
        )
        .bind(ch)
        .bind(available)
        .execute(pool)
        .await
        .context("updating mock_channels")?;
        Ok(MockChannelState { channel, available })
    }

    pub async fn get_delivery_by_key(pool: &PgPool, idempotency_key: &str) -> anyhow::Result<Option<DeliveryRecord>> {
        let row = sqlx::query(
            "SELECT id, alert_id, episode, person_id, channel, status, attempts, last_error, received_at, delivered_at, updated_at
             FROM deliveries
             WHERE idempotency_key = $1",
        )
        .bind(idempotency_key)
        .fetch_optional(pool)
        .await
        .context("fetching delivery by idempotency_key")?;
        row.as_ref().map(row_to_delivery).transpose()
    }

    pub async fn get_delivery_with_request(
        pool: &PgPool,
        delivery_id: Uuid,
    ) -> anyhow::Result<Option<(DeliveryRecord, DeliveryRequest)>> {
        let row = sqlx::query(
            "SELECT id, alert_id, episode, person_id, channel, status, attempts, last_error, request, received_at, delivered_at, updated_at
             FROM deliveries
             WHERE id = $1",
        )
        .bind(delivery_id)
        .fetch_optional(pool)
        .await
        .context("fetching delivery by id")?;

        match row {
            Some(r) => {
                let record = row_to_delivery(&r)?;
                let req_val: serde_json::Value = r.try_get("request")?;
                let req: DeliveryRequest = serde_json::from_value(req_val).context("deserializing delivery request")?;
                Ok(Some((record, req)))
            }
            None => Ok(None),
        }
    }

    pub async fn list_deliveries_for_alert(pool: &PgPool, alert_id: Uuid) -> anyhow::Result<Vec<DeliveryRecord>> {
        let rows = sqlx::query(
            "SELECT id, alert_id, episode, person_id, channel, status, attempts, last_error, received_at, delivered_at, updated_at
             FROM deliveries
             WHERE alert_id = $1
             ORDER BY received_at ASC, updated_at ASC",
        )
        .bind(alert_id)
        .fetch_all(pool)
        .await
        .context("listing deliveries for alert")?;
        rows.iter().map(row_to_delivery).collect()
    }

    pub async fn claim_delivery(pool: &PgPool, params: UpsertDelivery<'_>) -> anyhow::Result<ClaimOutcome> {
        let ch = channel_to_str(params.channel);
        let req_json = serde_json::to_value(params.request).context("serializing DeliveryRequest")?;
        let now = Utc::now();
        let lease_cutoff = now - chrono::Duration::seconds(PENDING_LEASE_SECS);

        let claimed = sqlx::query(
            "INSERT INTO deliveries (
                id, idempotency_key, alert_id, episode, person_id, channel,
                status, attempts, last_error, request, received_at, delivered_at, updated_at
             )
             VALUES ($1, $2, $3, $4, $5, $6, 'pending', $7, $8, $9, $10, NULL, $11)
             ON CONFLICT (idempotency_key) DO UPDATE SET
                status = 'pending',
                attempts = GREATEST(deliveries.attempts, EXCLUDED.attempts),
                last_error = EXCLUDED.last_error,
                request = EXCLUDED.request,
                updated_at = EXCLUDED.updated_at
             WHERE deliveries.status <> 'delivered'
               AND (
                    deliveries.status <> 'pending'
                    OR deliveries.attempts = 0
                    OR deliveries.updated_at < $12
               )
             RETURNING id, alert_id, episode, person_id, channel, status, attempts, last_error, received_at, delivered_at, updated_at",
        )
        .bind(params.request.delivery_id)
        .bind(params.idempotency_key)
        .bind(params.request.alert_id)
        .bind(params.request.episode)
        .bind(&params.request.person_id)
        .bind(ch)
        .bind(params.attempts)
        .bind(params.last_error)
        .bind(req_json)
        .bind(params.request.received_at)
        .bind(now)
        .bind(lease_cutoff)
        .fetch_optional(pool)
        .await
        .context("claiming delivery row")?;

        if let Some(row) = claimed {
            return Ok(ClaimOutcome::Claimed(row_to_delivery(&row)?));
        }

        let existing = get_delivery_by_key(pool, params.idempotency_key)
            .await?
            .ok_or_else(|| anyhow::anyhow!("delivery disappeared during claim: {}", params.idempotency_key))?;
        if existing.status == DeliveryStatus::Delivered {
            Ok(ClaimOutcome::AlreadyDelivered(existing))
        } else {
            Ok(ClaimOutcome::InProgress(existing))
        }
    }

    pub async fn upsert_delivery(pool: &PgPool, params: UpsertDelivery<'_>) -> anyhow::Result<DeliveryRecord> {
        let ch = channel_to_str(params.channel);
        let st = status_to_str(params.status);
        let req_json = serde_json::to_value(params.request).context("serializing DeliveryRequest")?;
        let now = Utc::now();

        let row = sqlx::query(
            "INSERT INTO deliveries (
                id, idempotency_key, alert_id, episode, person_id, channel,
                status, attempts, last_error, request, received_at, delivered_at, updated_at
             )
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
             ON CONFLICT (idempotency_key) DO UPDATE SET
                status = EXCLUDED.status,
                attempts = EXCLUDED.attempts,
                last_error = EXCLUDED.last_error,
                request = EXCLUDED.request,
                delivered_at = COALESCE(EXCLUDED.delivered_at, deliveries.delivered_at),
                updated_at = EXCLUDED.updated_at
             RETURNING id, alert_id, episode, person_id, channel, status, attempts, last_error, received_at, delivered_at, updated_at",
        )
        .bind(params.request.delivery_id)
        .bind(params.idempotency_key)
        .bind(params.request.alert_id)
        .bind(params.request.episode)
        .bind(&params.request.person_id)
        .bind(ch)
        .bind(st)
        .bind(params.attempts)
        .bind(params.last_error)
        .bind(req_json)
        .bind(params.request.received_at)
        .bind(params.delivered_at)
        .bind(now)
        .fetch_one(pool)
        .await
        .context("upserting delivery row")?;

        row_to_delivery(&row)
    }

    pub async fn insert_mock_outbox(
        pool: &PgPool,
        channel: Channel,
        to: &str,
        person_id: &str,
        title: &str,
        body: &str,
        alert_id: Option<Uuid>,
    ) -> anyhow::Result<MockMessage> {
        let ch = channel_to_str(channel);
        let row = sqlx::query(
            "INSERT INTO mock_outbox (channel, recipient, person_id, title, body, alert_id, sent_at)
             VALUES ($1, $2, $3, $4, $5, $6, now())
             RETURNING id, sent_at",
        )
        .bind(ch)
        .bind(to)
        .bind(person_id)
        .bind(title)
        .bind(body)
        .bind(alert_id)
        .fetch_one(pool)
        .await
        .context("inserting into mock_outbox")?;

        let id: i64 = row.try_get("id")?;
        let sent_at: Timestamp = row.try_get("sent_at")?;
        Ok(MockMessage {
            id,
            channel,
            to: to.to_string(),
            person_id: person_id.to_string(),
            title: title.to_string(),
            body: body.to_string(),
            alert_id,
            sent_at,
        })
    }

    pub async fn list_mock_outbox(pool: &PgPool) -> anyhow::Result<Vec<MockMessage>> {
        let rows = sqlx::query(
            "SELECT id, channel, recipient, person_id, title, body, alert_id, sent_at
             FROM mock_outbox
             ORDER BY sent_at DESC, id DESC",
        )
        .fetch_all(pool)
        .await
        .context("listing mock_outbox")?;

        rows.into_iter()
            .map(|r| {
                let ch: String = r.try_get("channel")?;
                Ok(MockMessage {
                    id: r.try_get("id")?,
                    channel: str_to_channel(&ch)?,
                    to: r.try_get("recipient")?,
                    person_id: r.try_get("person_id")?,
                    title: r.try_get("title")?,
                    body: r.try_get("body")?,
                    alert_id: r.try_get("alert_id")?,
                    sent_at: r.try_get("sent_at")?,
                })
            })
            .collect()
    }
}

impl DeliveryBackend for PgKafkaBackend {
    fn is_channel_available(&self, channel: Channel) -> BoxFuture<'_, anyhow::Result<bool>> {
        Box::pin(db::is_channel_available(&self.pool, channel))
    }

    fn get_delivery_by_key(&self, idempotency_key: &str) -> BoxFuture<'_, anyhow::Result<Option<DeliveryRecord>>> {
        let key = idempotency_key.to_string();
        Box::pin(async move { db::get_delivery_by_key(&self.pool, &key).await })
    }

    fn get_delivery_with_request(
        &self,
        delivery_id: Uuid,
    ) -> BoxFuture<'_, anyhow::Result<Option<(DeliveryRecord, DeliveryRequest)>>> {
        Box::pin(db::get_delivery_with_request(&self.pool, delivery_id))
    }

    fn claim_delivery<'a>(&'a self, params: UpsertDelivery<'a>) -> BoxFuture<'a, anyhow::Result<ClaimOutcome>> {
        Box::pin(async move { db::claim_delivery(&self.pool, params).await })
    }

    fn upsert_delivery<'a>(&'a self, params: UpsertDelivery<'a>) -> BoxFuture<'a, anyhow::Result<DeliveryRecord>> {
        Box::pin(db::upsert_delivery(&self.pool, params))
    }

    fn insert_mock_outbox<'a>(
        &'a self,
        channel: Channel,
        to: &'a str,
        person_id: &'a str,
        title: &'a str,
        body: &'a str,
        alert_id: Option<Uuid>,
    ) -> BoxFuture<'a, anyhow::Result<MockMessage>> {
        Box::pin(db::insert_mock_outbox(&self.pool, channel, to, person_id, title, body, alert_id))
    }

    fn publish_retry<'a>(&'a self, req: &'a DeliveryRequest) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            if let Some(producer) = &self.producer {
                let key = req.alert_id.to_string();
                common::kafka::publish(producer, topics::DELIVERIES_RETRY, &key, req).await?;
            }
            Ok(())
        })
    }

    fn publish_dlq<'a>(&'a self, req: &'a DeliveryRequest) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            if let Some(producer) = &self.producer {
                let key = req.alert_id.to_string();
                common::kafka::publish(producer, topics::DELIVERIES_DLQ, &key, req).await?;
            }
            Ok(())
        })
    }

    fn publish_requested<'a>(&'a self, req: &'a DeliveryRequest) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            if let Some(producer) = &self.producer {
                let key = req.alert_id.to_string();
                common::kafka::publish(producer, topics::DELIVERIES_REQUESTED, &key, req).await?;
            }
            Ok(())
        })
    }

    fn publish_ui_event<'a>(&'a self, event: &'a UiEvent) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            if let Some(producer) = &self.producer {
                let key = event
                    .alert_id
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "delivery".to_string());
                common::kafka::publish(producer, topics::UI_EVENTS, &key, event).await?;
            }
            Ok(())
        })
    }
}

// ============================================================================
// Implementacja In-Memory (do szybkich testów jednostkowych i deterministycznych symulacji)
// ============================================================================

#[derive(Debug, Default)]
struct InMemoryState {
    channels: BTreeMap<Channel, bool>,
    deliveries_by_key: HashMap<String, (DeliveryRecord, DeliveryRequest)>,
    key_order: Vec<String>,
    mock_outbox: Vec<MockMessage>,
    next_outbox_id: i64,
    retry_queue: Vec<DeliveryRequest>,
    dlq_queue: Vec<DeliveryRequest>,
    requested_queue: Vec<DeliveryRequest>,
    ui_events: Vec<UiEvent>,
}

#[derive(Clone, Default)]
pub struct InMemoryBackend {
    state: Arc<Mutex<InMemoryState>>,
}

impl InMemoryBackend {
    pub fn new() -> Self {
        let mut channels = BTreeMap::new();
        channels.insert(Channel::Sms, true);
        channels.insert(Channel::Phone, true);
        channels.insert(Channel::Email, true);
        channels.insert(Channel::Ui, true);
        Self {
            state: Arc::new(Mutex::new(InMemoryState {
                channels,
                next_outbox_id: 1,
                ..Default::default()
            })),
        }
    }

    pub fn set_channel_available(&self, channel: Channel, available: bool) {
        let mut s = self.state.lock().unwrap();
        s.channels.insert(channel, available);
    }

    pub fn deliveries(&self) -> Vec<DeliveryRecord> {
        let s = self.state.lock().unwrap();
        s.key_order
            .iter()
            .filter_map(|k| s.deliveries_by_key.get(k).map(|(rec, _)| rec.clone()))
            .collect()
    }

    pub fn mock_outbox(&self) -> Vec<MockMessage> {
        self.state.lock().unwrap().mock_outbox.clone()
    }

    pub fn take_retry_queue(&self) -> Vec<DeliveryRequest> {
        std::mem::take(&mut self.state.lock().unwrap().retry_queue)
    }

    pub fn retry_queue(&self) -> Vec<DeliveryRequest> {
        self.state.lock().unwrap().retry_queue.clone()
    }

    pub fn dlq_queue(&self) -> Vec<DeliveryRequest> {
        self.state.lock().unwrap().dlq_queue.clone()
    }

    pub fn requested_queue(&self) -> Vec<DeliveryRequest> {
        self.state.lock().unwrap().requested_queue.clone()
    }

    pub fn take_requested_queue(&self) -> Vec<DeliveryRequest> {
        std::mem::take(&mut self.state.lock().unwrap().requested_queue)
    }

    pub fn ui_events(&self) -> Vec<UiEvent> {
        self.state.lock().unwrap().ui_events.clone()
    }
}

impl DeliveryBackend for InMemoryBackend {
    fn is_channel_available(&self, channel: Channel) -> BoxFuture<'_, anyhow::Result<bool>> {
        let avail = self.state.lock().unwrap().channels.get(&channel).copied().unwrap_or(true);
        Box::pin(async move { Ok(avail) })
    }

    fn get_delivery_by_key(&self, idempotency_key: &str) -> BoxFuture<'_, anyhow::Result<Option<DeliveryRecord>>> {
        let rec = self
            .state
            .lock()
            .unwrap()
            .deliveries_by_key
            .get(idempotency_key)
            .map(|(r, _)| r.clone());
        Box::pin(async move { Ok(rec) })
    }

    fn get_delivery_with_request(
        &self,
        delivery_id: Uuid,
    ) -> BoxFuture<'_, anyhow::Result<Option<(DeliveryRecord, DeliveryRequest)>>> {
        let found = self
            .state
            .lock()
            .unwrap()
            .deliveries_by_key
            .values()
            .find(|(r, _)| r.id == delivery_id)
            .cloned();
        Box::pin(async move { Ok(found) })
    }

    fn claim_delivery<'a>(&'a self, params: UpsertDelivery<'a>) -> BoxFuture<'a, anyhow::Result<ClaimOutcome>> {
        let mut s = self.state.lock().unwrap();
        let now = Utc::now();
        let lease_cutoff = now - chrono::Duration::seconds(PENDING_LEASE_SECS);
        let key = params.idempotency_key.to_string();

        if let Some((existing, _)) = s.deliveries_by_key.get(&key) {
            if existing.status == DeliveryStatus::Delivered {
                let rec = existing.clone();
                return Box::pin(async move { Ok(ClaimOutcome::AlreadyDelivered(rec)) });
            }
            if existing.status == DeliveryStatus::Pending && existing.attempts > 0 && existing.updated_at >= lease_cutoff {
                let rec = existing.clone();
                return Box::pin(async move { Ok(ClaimOutcome::InProgress(rec)) });
            }
        }

        let (id, prev_attempts, existing_delivered_at) = match s.deliveries_by_key.get(&key) {
            Some((existing, _)) => (existing.id, existing.attempts, existing.delivered_at),
            None => {
                s.key_order.push(key.clone());
                (params.request.delivery_id, 0, None)
            }
        };

        let mut req_copy = params.request.clone();
        req_copy.delivery_id = id;

        let record = DeliveryRecord {
            id,
            alert_id: params.request.alert_id,
            episode: params.request.episode,
            person_id: params.request.person_id.clone(),
            channel: params.channel,
            status: DeliveryStatus::Pending,
            attempts: prev_attempts.max(params.attempts),
            last_error: params.last_error.map(ToOwned::to_owned),
            received_at: params.request.received_at,
            delivered_at: existing_delivered_at,
            updated_at: now,
        };

        s.deliveries_by_key.insert(key, (record.clone(), req_copy));
        Box::pin(async move { Ok(ClaimOutcome::Claimed(record)) })
    }

    fn upsert_delivery<'a>(&'a self, params: UpsertDelivery<'a>) -> BoxFuture<'a, anyhow::Result<DeliveryRecord>> {
        let mut s = self.state.lock().unwrap();
        let now = Utc::now();
        let key = params.idempotency_key.to_string();

        let (id, existing_delivered_at) = match s.deliveries_by_key.get(&key) {
            Some((existing, _)) => (existing.id, existing.delivered_at),
            None => {
                s.key_order.push(key.clone());
                (params.request.delivery_id, None)
            }
        };

        let mut req_copy = params.request.clone();
        req_copy.delivery_id = id;

        let record = DeliveryRecord {
            id,
            alert_id: params.request.alert_id,
            episode: params.request.episode,
            person_id: params.request.person_id.clone(),
            channel: params.channel,
            status: params.status,
            attempts: params.attempts,
            last_error: params.last_error.map(ToOwned::to_owned),
            received_at: params.request.received_at,
            delivered_at: params.delivered_at.or(existing_delivered_at),
            updated_at: now,
        };

        s.deliveries_by_key.insert(key, (record.clone(), req_copy));
        Box::pin(async move { Ok(record) })
    }

    fn insert_mock_outbox<'a>(
        &'a self,
        channel: Channel,
        to: &'a str,
        person_id: &'a str,
        title: &'a str,
        body: &'a str,
        alert_id: Option<Uuid>,
    ) -> BoxFuture<'a, anyhow::Result<MockMessage>> {
        let mut s = self.state.lock().unwrap();
        let id = s.next_outbox_id;
        s.next_outbox_id += 1;
        let msg = MockMessage {
            id,
            channel,
            to: to.to_string(),
            person_id: person_id.to_string(),
            title: title.to_string(),
            body: body.to_string(),
            alert_id,
            sent_at: Utc::now(),
        };
        s.mock_outbox.push(msg.clone());
        Box::pin(async move { Ok(msg) })
    }

    fn publish_retry<'a>(&'a self, req: &'a DeliveryRequest) -> BoxFuture<'a, anyhow::Result<()>> {
        self.state.lock().unwrap().retry_queue.push(req.clone());
        Box::pin(async { Ok(()) })
    }

    fn publish_dlq<'a>(&'a self, req: &'a DeliveryRequest) -> BoxFuture<'a, anyhow::Result<()>> {
        self.state.lock().unwrap().dlq_queue.push(req.clone());
        Box::pin(async { Ok(()) })
    }

    fn publish_requested<'a>(&'a self, req: &'a DeliveryRequest) -> BoxFuture<'a, anyhow::Result<()>> {
        self.state.lock().unwrap().requested_queue.push(req.clone());
        Box::pin(async { Ok(()) })
    }

    fn publish_ui_event<'a>(&'a self, event: &'a UiEvent) -> BoxFuture<'a, anyhow::Result<()>> {
        self.state.lock().unwrap().ui_events.push(event.clone());
        Box::pin(async { Ok(()) })
    }
}
