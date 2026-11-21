//! Dedup i cykl życia alertów (design D4): single writer per fingerprint.
//!
//! Jeden konsument czyta `events.normalized` i `alerts.commands` (oba z kluczem = fingerprint,
//! więc dany fingerprint trafia do jednej instancji). Stan trzymamy w cache; przejścia stanu
//! zapisujemy do Postgresa synchronnie (przed publikacją i commitem), a samo zliczanie
//! duplikatów jest wsadowe (flush co ~1 s, inkrement jako delta).
//!
//! Znane ograniczenie (akceptowane w D4): niezflushowane inkrementy licznika giną przy awarii
//! procesu, a powtórne dostarczenie komendy ack/resolve po awarii między zapisem a publikacją
//! nie opublikuje `AlertChanged` ponownie (dla zdarzeń otwierających/zamykających to obsługujemy
//! porównaniem `event_id` z ostatnim zdarzeniem alertu).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use common::db::sqlx::{PgPool, Row};
use common::kafka::rdkafka::config::ClientConfig;
use common::kafka::rdkafka::consumer::{
    BaseConsumer, CommitMode, Consumer, ConsumerContext, Rebalance, StreamConsumer,
};
use common::kafka::rdkafka::message::{BorrowedMessage, Message};
use common::kafka::rdkafka::producer::FutureProducer;
use common::kafka::rdkafka::ClientContext;
use contracts::{
    topics, Actor, AlertChanged, AlertCommand, AlertCommandKind, AlertState, CanonicalEvent,
    Severity, Timestamp, Transition, UiEvent, UiEventKind,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{Shutdown, Topics};

/// Po przejęciu/utracie partycji ustawia flagę; pętla flushuje i czyści cache.
struct RebalanceContext {
    rebalanced: Arc<AtomicBool>,
}

impl ClientContext for RebalanceContext {}

impl ConsumerContext for RebalanceContext {
    fn post_rebalance(&self, _base_consumer: &BaseConsumer<Self>, _rebalance: &Rebalance<'_>) {
        self.rebalanced.store(true, Ordering::SeqCst);
    }
}

const MAX_PERMANENT_FLUSH_FAILURES: u32 = 5;

struct AlertEntry {
    alert_id: Uuid,
    episode: i32,
    state: AlertState,
    severity: Severity,
    count: i64,
    last_seen: Timestamp,
    last_event: Option<CanonicalEvent>,
    /// Inkrementy licznika jeszcze niezapisane w bazie.
    pending_delta: i64,
    dirty: bool,
    /// Liczba kolejnych trwałych błędów flusha.
    flush_failures: u32,
}

impl AlertEntry {
    /// Rejestruje błąd flusha; zwraca `true` jeśli przekroczono limit trwałych błędów i wyczyszczono `dirty`.
    fn record_flush_failure(&mut self, permanent: bool) -> bool {
        if !permanent {
            return false;
        }
        self.flush_failures = self.flush_failures.saturating_add(1);
        if self.flush_failures > MAX_PERMANENT_FLUSH_FAILURES {
            self.dirty = false;
            self.pending_delta = 0;
            self.flush_failures = 0;
            return true;
        }
        false
    }
}

fn sanitize_event(ev: &mut CanonicalEvent) {
    crate::mapping::strip_nuls_str(&mut ev.schema);
    crate::mapping::strip_nuls_str(&mut ev.source);
    crate::mapping::strip_nuls_str(&mut ev.title);
    crate::mapping::strip_nuls_str(&mut ev.fingerprint);
    ev.fingerprint = crate::mapping::bound_fingerprint(std::mem::take(&mut ev.fingerprint));
    crate::mapping::strip_nuls_value(&mut ev.raw);
    for val in ev.fields.values_mut() {
        crate::mapping::strip_nuls_value(val);
    }
}

fn enum_str<T: Serialize>(v: &T) -> String {
    match serde_json::to_value(v) {
        Ok(Value::String(s)) => s,
        _ => String::new(),
    }
}

fn enum_parse<T: DeserializeOwned>(s: &str) -> Result<T> {
    serde_json::from_value(Value::String(s.to_string()))
        .with_context(|| format!("unknown enum value `{s}`"))
}

struct Dedup {
    pool: PgPool,
    producer: FutureProducer,
    topics: Topics,
    cache: HashMap<String, AlertEntry>,
}

impl Dedup {
    /// Publikacja ponawiana do skutku – stan w bazie jest już zapisany, więc nie wolno jej zgubić.
    async fn publish<T: Serialize>(&self, topic: &str, key: &str, msg: &T) {
        while let Err(e) = common::kafka::publish(&self.producer, topic, key, msg).await {
            tracing::warn!(topic, error = %e, "dedup: publish failed, retrying");
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    async fn load(&mut self, fingerprint: &str) -> Result<bool> {
        if self.cache.contains_key(fingerprint) {
            return Ok(true);
        }
        let row = common::db::sqlx::query(
            "SELECT id, state, severity, count, episode, last_seen, last_event FROM alerts WHERE fingerprint = $1",
        )
        .bind(fingerprint)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else { return Ok(false) };
        let alert_id: Uuid = row.get("id");
        let last_event: Option<Value> = row.get("last_event");
        let last_event = match last_event {
            Some(v) => match serde_json::from_value(v) {
                Ok(ev) => Some(ev),
                Err(e) => {
                    tracing::warn!(%alert_id, fingerprint, error = %e, "dedup: failed to deserialize last_event from db");
                    None
                }
            },
            None => None,
        };
        let entry = AlertEntry {
            alert_id,
            episode: row.get("episode"),
            state: enum_parse(&row.get::<String, _>("state"))?,
            severity: enum_parse(&row.get::<String, _>("severity"))?,
            count: row.get("count"),
            last_seen: row.get("last_seen"),
            last_event,
            pending_delta: 0,
            dirty: false,
            flush_failures: 0,
        };
        self.cache.insert(fingerprint.to_string(), entry);
        Ok(true)
    }

    /// Zapisuje nagromadzone inkrementy jednego alertu i wysyła `AlertCount` na `ui.events`.
    async fn flush_one(&mut self, fingerprint: &str) -> Result<()> {
        let Some(e) = self.cache.get(fingerprint) else {
            return Ok(());
        };
        if !e.dirty {
            return Ok(());
        }
        let (alert_id, episode, pending_delta, last_seen) =
            (e.alert_id, e.episode, e.pending_delta, e.last_seen);
        let mut last_event_json = e
            .last_event
            .as_ref()
            .map(serde_json::to_value)
            .transpose()?;
        if let Some(ref mut v) = last_event_json {
            crate::mapping::strip_nuls_value(v);
        }

        let mut tx = self.pool.begin().await?;
        let row = common::db::sqlx::query(
            "UPDATE alerts SET count = count + $1, last_seen = GREATEST(last_seen, $2), last_event = COALESCE($3, last_event),
                    updated_at = now()
             WHERE id = $4 AND episode = $5 RETURNING count",
        )
        .bind(pending_delta)
        .bind(last_seen)
        .bind(&last_event_json)
        .bind(alert_id)
        .bind(episode)
        .fetch_optional(&mut *tx)
        .await?;
        let new_count: Option<i64> = if let Some(row) = &row {
            let count: i64 = row.get("count");
            common::db::sqlx::query(
                "UPDATE alert_episodes SET count = count + $1 WHERE alert_id = $2 AND episode = $3",
            )
            .bind(pending_delta)
            .bind(alert_id)
            .bind(episode)
            .execute(&mut *tx)
            .await?;
            Some(count)
        } else {
            None
        };
        tx.commit().await?;

        let e = self.cache.get_mut(fingerprint).expect("checked above");
        if let Some(c) = new_count {
            e.count = c;
        }
        e.pending_delta = 0;
        e.dirty = false;
        e.flush_failures = 0;
        let ui = UiEvent {
            alert_id: Some(e.alert_id),
            kind: UiEventKind::AlertCount,
            payload: json!({"count": e.count, "last_seen": e.last_seen}),
            at: chrono::Utc::now(),
        };
        let key = e.alert_id.to_string();
        if let Err(err) =
            common::kafka::publish(&self.producer, &self.topics.ui_events, &key, &ui).await
        {
            tracing::warn!(error = %err, "dedup: ui count publish failed");
        }
        Ok(())
    }

    async fn flush_all(&mut self) {
        let dirty: Vec<String> = self
            .cache
            .iter()
            .filter(|(_, e)| e.dirty)
            .map(|(k, _)| k.clone())
            .collect();
        for fp in dirty {
            if let Err(e) = self.flush_one(&fp).await {
                let permanent = crate::is_permanent_pg_error(&e);
                if let Some(entry) = self.cache.get_mut(&fp) {
                    if entry.record_flush_failure(permanent) {
                        tracing::error!(
                            fingerprint = %fp,
                            alert_id = %entry.alert_id,
                            error = %e,
                            "dedup: permanent flush failure threshold exceeded, clearing dirty flag"
                        );
                        continue;
                    }
                }
                tracing::warn!(fingerprint = %fp, error = %e, "dedup: flush failed, will retry");
            }
        }
    }

    fn changed(
        &self,
        fp: &str,
        transition: Transition,
        actor: Option<Actor>,
        event: CanonicalEvent,
    ) -> AlertChanged {
        let e = &self.cache[fp];
        AlertChanged {
            alert_id: e.alert_id,
            fingerprint: fp.to_string(),
            episode: e.episode,
            transition,
            state: e.state,
            severity: e.severity,
            count: e.count,
            event,
            actor,
            at: chrono::Utc::now(),
        }
    }

    async fn emit(&self, ch: &AlertChanged) {
        self.publish(&self.topics.alerts_changed, &ch.alert_id.to_string(), ch)
            .await;
    }

    async fn handle_event(&mut self, mut ev: CanonicalEvent) -> Result<()> {
        sanitize_event(&mut ev);
        let fp = ev.fingerprint.clone();
        if !self.load(&fp).await? {
            if ev.resolved {
                return Ok(());
            }
            return self.open(ev).await;
        }
        let (state, last_id, count) = {
            let e = &self.cache[&fp];
            (e.state, e.last_event.as_ref().map(|l| l.event_id), e.count)
        };
        if last_id == Some(ev.event_id) {
            let transition = match state {
                AlertState::Open if count == 1 => Some(Transition::Opened),
                AlertState::Reopened if count == 1 => Some(Transition::Reopened),
                AlertState::Resolved if ev.resolved => Some(Transition::Resolved),
                _ => None,
            };
            if let Some(t) = transition {
                let actor = (t == Transition::Resolved).then_some(Actor::Source);
                let ch = self.changed(&fp, t, actor, ev);
                self.emit(&ch).await;
            }
            return Ok(());
        }
        match state {
            AlertState::Open | AlertState::Acked | AlertState::Reopened if ev.resolved => {
                self.transition(
                    &fp,
                    Transition::Resolved,
                    AlertState::Resolved,
                    Actor::Source,
                    Some(ev),
                )
                .await
            }
            AlertState::Open | AlertState::Acked | AlertState::Reopened => {
                let e = self.cache.get_mut(&fp).expect("loaded");
                e.count += 1;
                e.pending_delta += 1;
                e.last_seen = ev.received_at;
                e.last_event = Some(ev);
                e.dirty = true;
                Ok(())
            }
            AlertState::Resolved if ev.resolved => Ok(()),
            AlertState::Resolved => self.reopen(&fp, ev).await,
        }
    }

    async fn open(&mut self, ev: CanonicalEvent) -> Result<()> {
        let id = Uuid::now_v7();
        let mut tx = self.pool.begin().await?;
        let inserted = common::db::sqlx::query(
            "INSERT INTO alerts (id, fingerprint, state, severity, title, schema, episode, count, first_seen, last_seen, last_event, updated_at)
             VALUES ($1, $2, 'open', $3, $4, $5, 1, 1, $6, $6, $7, now()) ON CONFLICT (fingerprint) DO NOTHING",
        )
        .bind(id)
        .bind(&ev.fingerprint)
        .bind(enum_str(&ev.severity))
        .bind(&ev.title)
        .bind(&ev.schema)
        .bind(ev.received_at)
        .bind(serde_json::to_value(&ev)?)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if inserted == 0 {
            tx.rollback().await?;
            return Box::pin(self.handle_event(ev)).await;
        }
        common::db::sqlx::query("INSERT INTO alert_episodes (alert_id, episode, opened_at, count) VALUES ($1, 1, $2, 1)")
            .bind(id)
            .bind(ev.received_at)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        let fp = ev.fingerprint.clone();
        self.cache.insert(
            fp.clone(),
            AlertEntry {
                alert_id: id,
                episode: 1,
                state: AlertState::Open,
                severity: ev.severity,
                count: 1,
                last_seen: ev.received_at,
                last_event: Some(ev.clone()),
                pending_delta: 0,
                dirty: false,
                flush_failures: 0,
            },
        );
        let ch = self.changed(&fp, Transition::Opened, None, ev);
        self.emit(&ch).await;
        Ok(())
    }

    async fn reopen(&mut self, fp: &str, ev: CanonicalEvent) -> Result<()> {
        let (id, old_ep) = {
            let e = &self.cache[fp];
            (e.alert_id, e.episode)
        };
        let mut tx = self.pool.begin().await?;
        let n = common::db::sqlx::query(
            "UPDATE alerts SET state = 'reopened', episode = episode + 1, count = 1, first_seen = $1, last_seen = $1,
                    severity = $2, title = $3, last_event = $4, acked_by = NULL, resolved_by = NULL, updated_at = now()
             WHERE id = $5 AND episode = $6 AND state = 'resolved'",
        )
        .bind(ev.received_at)
        .bind(enum_str(&ev.severity))
        .bind(&ev.title)
        .bind(serde_json::to_value(&ev)?)
        .bind(id)
        .bind(old_ep)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if n == 0 {
            self.cache.remove(fp);
            return Err(anyhow!("alert {id} changed concurrently, reloading"));
        }
        common::db::sqlx::query("INSERT INTO alert_episodes (alert_id, episode, opened_at, count) VALUES ($1, $2, $3, 1)")
            .bind(id)
            .bind(old_ep + 1)
            .bind(ev.received_at)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        let e = self.cache.get_mut(fp).expect("loaded");
        e.episode = old_ep + 1;
        e.state = AlertState::Reopened;
        e.severity = ev.severity;
        e.count = 1;
        e.last_seen = ev.received_at;
        e.last_event = Some(ev.clone());
        e.pending_delta = 0;
        e.dirty = false;
        e.flush_failures = 0;
        let ch = self.changed(fp, Transition::Reopened, None, ev);
        self.emit(&ch).await;
        Ok(())
    }

    /// Ack / Resolve: zapis do bazy, aktualizacja cache, publikacja `AlertChanged`.
    async fn transition(
        &mut self,
        fp: &str,
        t: Transition,
        new: AlertState,
        actor: Actor,
        ev: Option<CanonicalEvent>,
    ) -> Result<()> {
        self.flush_one(fp).await?;
        let (id, ep, old_state) = {
            let e = &self.cache[fp];
            (e.alert_id, e.episode, e.state)
        };
        let mut actor_json = serde_json::to_value(&actor)?;
        crate::mapping::strip_nuls_value(&mut actor_json);
        let mut ev_json = ev.as_ref().map(serde_json::to_value).transpose()?;
        if let Some(ref mut v) = ev_json {
            crate::mapping::strip_nuls_value(v);
        }
        let (alerts_sql, ep_sql) = match t {
            Transition::Acked => (
                "UPDATE alerts SET state = 'acked', acked_by = $1, last_event = COALESCE($2, last_event), updated_at = now()
                 WHERE id = $3 AND episode = $4 AND state = $5",
                "UPDATE alert_episodes SET acked_at = now(), acked_by = $1 WHERE alert_id = $2 AND episode = $3",
            ),
            _ => (
                "UPDATE alerts SET state = 'resolved', resolved_by = $1, last_event = COALESCE($2, last_event), updated_at = now()
                 WHERE id = $3 AND episode = $4 AND state = $5",
                "UPDATE alert_episodes SET resolved_at = now(), resolved_by = $1 WHERE alert_id = $2 AND episode = $3",
            ),
        };
        let mut tx = self.pool.begin().await?;
        let n = common::db::sqlx::query(alerts_sql)
            .bind(&actor_json)
            .bind(&ev_json)
            .bind(id)
            .bind(ep)
            .bind(enum_str(&old_state))
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if n == 0 {
            self.cache.remove(fp);
            return Err(anyhow!("alert {id} changed concurrently, reloading"));
        }
        common::db::sqlx::query(ep_sql)
            .bind(&actor_json)
            .bind(id)
            .bind(ep)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;

        let e = self.cache.get_mut(fp).expect("loaded");
        e.state = new;
        if let Some(ev) = ev {
            e.last_event = Some(ev);
        }
        let Some(event) = e.last_event.clone() else {
            return Ok(());
        };
        let ch = self.changed(fp, t, Some(actor), event);
        self.emit(&ch).await;
        Ok(())
    }

    async fn handle_command(&mut self, cmd: AlertCommand) -> Result<()> {
        if !self.load(&cmd.fingerprint).await?
            || self.cache[&cmd.fingerprint].alert_id != cmd.alert_id
        {
            tracing::warn!(alert_id = %cmd.alert_id, "dedup: command for unknown alert, ignored");
            return Ok(());
        }
        let state = self.cache[&cmd.fingerprint].state;
        let (t, new) = match (cmd.kind, state) {
            (AlertCommandKind::Ack, AlertState::Open | AlertState::Reopened) => {
                (Transition::Acked, AlertState::Acked)
            }
            (
                AlertCommandKind::Resolve,
                AlertState::Open | AlertState::Acked | AlertState::Reopened,
            ) => (Transition::Resolved, AlertState::Resolved),
            _ => return Ok(()),
        };
        self.transition(&cmd.fingerprint, t, new, cmd.actor, None)
            .await
    }

    async fn handle(&mut self, msg: &BorrowedMessage<'_>) -> Result<()> {
        if msg.topic() == self.topics.alerts_commands {
            match common::kafka::decode::<AlertCommand>(msg) {
                Ok(c) => self.handle_command(c).await,
                Err(e) => {
                    tracing::warn!(error = %e, "dedup: undecodable command, skipped");
                    Ok(())
                }
            }
        } else {
            match common::kafka::decode::<CanonicalEvent>(msg) {
                Ok(ev) => self.handle_event(ev).await,
                Err(e) => {
                    tracing::warn!(error = %e, "dedup: undecodable event, skipped");
                    Ok(())
                }
            }
        }
    }

    /// Po zmianie przypisania partycji: najpierw flush, potem czyszczenie cache
    /// (stan dla przejętych partycji doczytujemy leniwie z Postgresa).
    async fn on_rebalance(&mut self) {
        self.flush_all().await;
        self.cache.clear();
    }
}

/// Konsument `events.normalized` + `alerts.commands` w grupie `lifecycle-dedup`; działa do sygnału zatrzymania.
pub async fn run_dedup(
    brokers: String,
    pool: PgPool,
    producer: FutureProducer,
    shutdown: Shutdown,
) -> Result<()> {
    run_dedup_with(
        topics::groups::LIFECYCLE_DEDUP,
        Topics::default(),
        brokers,
        pool,
        producer,
        shutdown,
    )
    .await
}

/// Jak `run_dedup`, ale z podaną grupą i topikami (testy używają własnych, izolowanych).
pub async fn run_dedup_with(
    group: &str,
    topics: Topics,
    brokers: String,
    pool: PgPool,
    producer: FutureProducer,
    mut shutdown: Shutdown,
) -> Result<()> {
    let rebalanced = Arc::new(AtomicBool::new(false));
    let consumer: StreamConsumer<RebalanceContext> = ClientConfig::new()
        .set("bootstrap.servers", &brokers)
        .set("group.id", group)
        .set("enable.auto.commit", "false")
        .set("auto.offset.reset", "earliest")
        .set("session.timeout.ms", "10000")
        .create_with_context(RebalanceContext {
            rebalanced: rebalanced.clone(),
        })
        .context("creating dedup consumer")?;
    consumer.subscribe(&[
        topics.events_normalized.as_str(),
        topics.alerts_commands.as_str(),
    ])?;

    let mut d = Dedup {
        pool,
        producer,
        topics,
        cache: HashMap::new(),
    };
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        if rebalanced.swap(false, Ordering::SeqCst) {
            d.on_rebalance().await;
        }
        tokio::select! {
            _ = shutdown.changed() => {
                d.flush_all().await;
                return Ok(());
            }
            _ = tick.tick() => {
                d.flush_all().await;
                if d.cache.len() > 50_000 {
                    d.cache.retain(|_, e| e.dirty);
                }
            }
            m = consumer.recv() => {
                let msg = match m {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::warn!(error = %e, "dedup: kafka error");
                        tokio::select! {
                            _ = shutdown.changed() => {
                                d.flush_all().await;
                                return Ok(());
                            }
                            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                        }
                        continue;
                    }
                };
                if rebalanced.swap(false, Ordering::SeqCst) {
                    d.on_rebalance().await;
                }
                let mut permanent_failures = 0u32;
                while let Err(e) = d.handle(&msg).await {
                    if crate::is_permanent_pg_error(&e) {
                        permanent_failures += 1;
                        if permanent_failures >= crate::MAX_PERMANENT_RETRIES {
                            tracing::error!(error = %e, retries = permanent_failures, "dedup: permanent postgres error, skipping message");
                            break;
                        }
                    }
                    tracing::warn!(error = %e, "dedup: processing failed, retrying");
                    tokio::select! {
                        _ = shutdown.changed() => {
                            d.flush_all().await;
                            return Ok(());
                        }
                        _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                    }
                }
                if let Err(e) = consumer.commit_message(&msg, CommitMode::Async) {
                    tracing::warn!(error = %e, "dedup: commit failed");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entry() -> AlertEntry {
        AlertEntry {
            alert_id: Uuid::new_v4(),
            episode: 1,
            state: AlertState::Open,
            severity: Severity::Error,
            count: 2,
            last_seen: chrono::Utc::now(),
            last_event: None,
            pending_delta: 1,
            dirty: true,
            flush_failures: 0,
        }
    }

    #[test]
    fn dirty_flag_cleared_after_permanent_flush_failure_threshold() {
        let mut entry = sample_entry();
        for _ in 0..10 {
            assert!(!entry.record_flush_failure(false));
            assert!(entry.dirty);
        }
        for _ in 0..MAX_PERMANENT_FLUSH_FAILURES {
            assert!(!entry.record_flush_failure(true));
            assert!(entry.dirty);
        }
        assert!(entry.record_flush_failure(true));
        assert!(!entry.dirty);
        assert_eq!(entry.pending_delta, 0);
        assert_eq!(entry.flush_failures, 0);
    }

    #[test]
    fn sanitize_event_strips_nuls_and_bounds_fingerprint() {
        let mut ev = CanonicalEvent {
            event_id: Uuid::new_v4(),
            schema: "s\0v1".into(),
            source: "http\0".into(),
            severity: Severity::Warning,
            title: "hello\0world".into(),
            fingerprint: format!("fp\0|{}", "x".repeat(1000)),
            resolved: false,
            fields: serde_json::Map::from_iter([("k".into(), json!("v\0"))]),
            raw: json!({"a\0": "b\0"}),
            received_at: chrono::Utc::now(),
            mapping_version: 1,
        };
        sanitize_event(&mut ev);
        assert_eq!(ev.schema, "sv1");
        assert_eq!(ev.source, "http");
        assert_eq!(ev.title, "helloworld");
        assert!(!ev.fingerprint.contains('\0'));
        assert!(ev.fingerprint.len() <= crate::mapping::MAX_FINGERPRINT_BYTES);
        assert_eq!(ev.fields["k"], json!("v"));
        assert_eq!(ev.raw, json!({"a": "b"}));
    }
}
