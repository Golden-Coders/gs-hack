//! SSE `/stream` endpoint.
//!
//! Consumes Kafka topics `alerts.changed` and `ui.events` (using a per-instance
//! group `api-sse-<uuid>`, starting from `latest` without committing offsets;
//! after restart UI catches up via `GET /alerts?since=`) and fans events out via
//! `tokio::sync::broadcast`. Format matches `contracts/openapi.yaml`:
//! `event: hello` (`data: {}`), `event: alert_changed` (`AlertChanged`),
//! `event: ui_event` (`UiEvent`).
//!
//! The consumer starts lazily on the first client. Slow clients (`RecvError::Lagged`)
//! skip missed events without blocking others.
//!
//! `?team=<id>` filter: an alert belongs to a team if it matches an enabled rule
//! whose `team` or team recipient matches that team. `ui_event` with `alert_id`
//! inherits the alert's teams; `ui_event` without `alert_id` (e.g. `unmapped`,
//! `config_changed`) goes to all clients. `ui_event` with an unknown `alert_id`
//! goes only to unfiltered clients.

use std::{
    collections::HashMap,
    convert::Infallible,
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

use axum::{
    extract::{Query, State},
    response::sse::{Event, KeepAlive, Sse},
};
use common::db::sqlx::{self, postgres::PgPool, types::Uuid, Row};
use common::kafka::rdkafka::{
    config::ClientConfig,
    consumer::{Consumer, StreamConsumer},
    Message,
};
use contracts::{topics, AlertChanged, CanonicalEvent, UiEvent};
use futures::{stream, Stream, StreamExt};
use serde::Deserialize;
use tokio::sync::{broadcast, watch};

use crate::rules_eval::{load_enabled_rules, teams_for_event, CompiledRule};
use crate::AppState;

/// Pojemność bufora broadcast; klient dalej w tyle dostaje `Lagged` i pomija zaległe.
const CHANNEL_CAPACITY: usize = 1024;
const HEARTBEAT: Duration = Duration::from_secs(15);
/// Jak często przeładowujemy reguły do filtra zespołów.
const RULES_TTL: Duration = Duration::from_secs(5);
/// Limit pamięci podręcznej alert -> zespoły (po przekroczeniu czyścimy całość).
const ALERT_CACHE_MAX: usize = 10_000;

/// Zdarzenie gotowe do wysłania: nazwa, JSON i zespoły (`None` = dla wszystkich).
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub name: &'static str,
    pub data: String,
    pub teams: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
pub struct StreamQuery {
    team: Option<String>,
}

static HUB: OnceLock<broadcast::Sender<Arc<Item>>> = OnceLock::new();
/// `true` po sygnale zamknięcia; zadanie nasłuchujące startuje przy pierwszym kliencie.
static SHUTDOWN: OnceLock<watch::Receiver<bool>> = OnceLock::new();

fn shutdown_rx() -> watch::Receiver<bool> {
    SHUTDOWN
        .get_or_init(|| {
            let (tx, rx) = watch::channel(false);
            tokio::spawn(async move {
                common::shutdown_signal().await;
                let _ = tx.send(true);
            });
            rx
        })
        .clone()
}

/// Kończy strumień, gdy `shutdown` przejdzie na `true` (albo nadawca zniknie).
fn until_shutdown<S: Stream>(s: S, mut shutdown: watch::Receiver<bool>) -> impl Stream<Item = S::Item> {
    s.take_until(async move {
        let _ = shutdown.wait_for(|v| *v).await;
    })
}

pub async fn sse(
    State(st): State<AppState>,
    Query(q): Query<StreamQuery>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let shutdown = shutdown_rx();
    let rx = HUB.get_or_init(|| start(st)).subscribe();
    let hello = stream::once(async { Ok(Event::default().event("hello").data("{}")) });
    let events = hello.chain(client_stream(rx, q.team.filter(|t| !t.is_empty())));
    Sse::new(until_shutdown(events, shutdown))
        .keep_alive(KeepAlive::new().interval(HEARTBEAT))
}

/// Strumień jednego klienta: filtr po zespole, pominięcie zaległych przy `Lagged`.
fn client_stream(
    rx: broadcast::Receiver<Arc<Item>>,
    team: Option<String>,
) -> impl Stream<Item = Result<Event, Infallible>> {
    stream::unfold((rx, team), |(mut rx, team)| async move {
        loop {
            match rx.recv().await {
                Ok(item) if visible(&item, team.as_deref()) => {
                    let ev = Event::default().event(item.name).data(&item.data);
                    return Some((Ok(ev), (rx, team)));
                }
                Ok(_) => continue,
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(skipped = n, "sse client lagged, skipping events");
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    })
}

fn visible(item: &Item, team: Option<&str>) -> bool {
    match (team, &item.teams) {
        (None, _) | (_, None) => true,
        (Some(t), Some(teams)) => teams.iter().any(|x| x == t),
    }
}

/// Zdekodowana wiadomość z Kafki.
#[derive(Debug)]
enum Decoded {
    Alert(Box<AlertChanged>),
    Ui(UiEvent),
}

fn decode(topic: &str, payload: &[u8]) -> anyhow::Result<Decoded> {
    Ok(match topic {
        topics::ALERTS_CHANGED => Decoded::Alert(Box::new(serde_json::from_slice(payload)?)),
        topics::UI_EVENTS => Decoded::Ui(serde_json::from_slice(payload)?),
        other => anyhow::bail!("unexpected topic {other}"),
    })
}

/// Nazwa zdarzenia SSE i dane JSON zgodnie z `contracts/openapi.yaml`.
fn render(d: &Decoded) -> (&'static str, String) {
    match d {
        Decoded::Alert(a) => ("alert_changed", serde_json::to_string(a).expect("AlertChanged serializes")),
        Decoded::Ui(u) => ("ui_event", serde_json::to_string(u).expect("UiEvent serializes")),
    }
}

fn start(st: AppState) -> broadcast::Sender<Arc<Item>> {
    let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
    let task_tx = tx.clone();
    tokio::spawn(async move {
        let mut teams = TeamResolver::new(st.db.clone());
        loop {
            if let Err(e) = run_consumer(&st.config.kafka_brokers, &task_tx, &mut teams).await {
                tracing::error!(error = %e, "sse consumer failed, retrying in 5 s");
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
    tx
}

async fn run_consumer(
    brokers: &str,
    tx: &broadcast::Sender<Arc<Item>>,
    teams: &mut TeamResolver,
) -> anyhow::Result<()> {
    // Własna konfiguracja zamiast `common::kafka::consumer`: tu chcemy `latest`
    // i nie commitujemy offsetów (grupa jest jednorazowa, per instancja).
    let group = format!("api-sse-{}", Uuid::new_v4());
    let consumer: StreamConsumer = ClientConfig::new()
        .set("bootstrap.servers", brokers)
        .set("group.id", &group)
        .set("enable.auto.commit", "false")
        .set("auto.offset.reset", "latest")
        .set("session.timeout.ms", "10000")
        .create()?;
    consumer.subscribe(&[topics::ALERTS_CHANGED, topics::UI_EVENTS])?;
    tracing::info!(%group, "sse consumer started");
    loop {
        let msg = consumer.recv().await?;
        let Some(payload) = msg.payload() else { continue };
        let decoded = match decode(msg.topic(), payload) {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(topic = msg.topic(), error = %e, "sse: skipping undecodable message");
                continue;
            }
        };
        let (name, data) = render(&decoded);
        let item_teams = teams.resolve(&decoded).await;
        // Błąd = brak podłączonych klientów; to nie problem.
        let _ = tx.send(Arc::new(Item { name, data, teams: item_teams }));
    }
}

struct TeamResolver {
    db: PgPool,
    rules: Vec<CompiledRule>,
    loaded_at: Option<Instant>,
    alerts: HashMap<Uuid, Vec<String>>,
}

impl TeamResolver {
    fn new(db: PgPool) -> Self {
        Self { db, rules: Vec::new(), loaded_at: None, alerts: HashMap::new() }
    }

    async fn refresh_rules(&mut self) {
        if self.loaded_at.is_some_and(|t| t.elapsed() < RULES_TTL) {
            return;
        }
        self.rules = load_enabled_rules(&self.db).await;
        self.loaded_at = Some(Instant::now());
    }

    fn remember(&mut self, id: Uuid, teams: Vec<String>) {
        if self.alerts.len() >= ALERT_CACHE_MAX {
            self.alerts.clear();
        }
        self.alerts.insert(id, teams);
    }

    /// `None` = zdarzenie dla wszystkich (`ui_event` bez `alert_id`). Nieznany alert
    /// (`ui_event` z `alert_id`, którego nie ma w bazie albo lookup się nie udał)
    /// = pusta lista, czyli tylko klienci bez filtra `?team=`.
    async fn resolve(&mut self, d: &Decoded) -> Option<Vec<String>> {
        match d {
            Decoded::Alert(a) => {
                self.refresh_rules().await;
                let teams = teams_for_event(&self.rules, &a.event);
                self.remember(a.alert_id, teams.clone());
                Some(teams)
            }
            Decoded::Ui(u) => {
                let id = u.alert_id?;
                if let Some(t) = self.alerts.get(&id) {
                    return Some(t.clone());
                }
                let row = sqlx::query("SELECT last_event FROM alerts WHERE id = $1")
                    .bind(id)
                    .fetch_optional(&self.db)
                    .await
                    .ok()
                    .flatten();
                let event = row.and_then(|r| serde_json::from_value::<CanonicalEvent>(r.get("last_event")).ok());
                let Some(event) = event else {
                    // Nie zapamiętujemy: alert może pojawić się w bazie za chwilę.
                    return Some(Vec::new());
                };
                self.refresh_rules().await;
                let teams = teams_for_event(&self.rules, &event);
                self.remember(id, teams.clone());
                Some(teams)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;

    const ALERT: &str = include_str!("../../contracts/fixtures/topics/alerts.changed.json");
    const UI: &str = include_str!("../../contracts/fixtures/topics/ui.events.json");

    fn item(name: &'static str, data: &str, teams: Option<&[&str]>) -> Arc<Item> {
        Arc::new(Item {
            name,
            data: data.to_string(),
            teams: teams.map(|t| t.iter().map(|s| s.to_string()).collect()),
        })
    }

    /// Renderuje strumień klienta do tekstu odpowiedzi HTTP (tak jak zobaczy go przeglądarka).
    async fn body(items: Vec<Arc<Item>>, team: Option<&str>) -> String {
        let (tx, rx) = broadcast::channel(16);
        for i in items {
            tx.send(i).unwrap();
        }
        drop(tx);
        let resp = Sse::new(client_stream(rx, team.map(String::from))).into_response();
        assert_eq!(resp.headers()["content-type"], "text/event-stream");
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[test]
    fn decodes_and_renders_fixtures_per_openapi() {
        let d = decode(topics::ALERTS_CHANGED, ALERT.as_bytes()).unwrap();
        let (name, data) = render(&d);
        assert_eq!(name, "alert_changed");
        let back: AlertChanged = serde_json::from_str(&data).unwrap();
        assert_eq!(back.fingerprint, "workflow.v1|order-settlement|validate-counterparty|TIMEOUT");
        assert!(!data.contains('\n'), "dane SSE w jednej linii");

        let d = decode(topics::UI_EVENTS, UI.as_bytes()).unwrap();
        let (name, data) = render(&d);
        assert_eq!(name, "ui_event");
        serde_json::from_str::<UiEvent>(&data).unwrap();

        assert!(decode(topics::ALERTS_CHANGED, b"{not json").is_err());
        assert!(decode("other", ALERT.as_bytes()).is_err());
    }

    #[tokio::test]
    async fn sse_wire_format() {
        let out = body(vec![item("alert_changed", r#"{"a":1}"#, None), item("ui_event", r#"{"b":2}"#, None)], None).await;
        assert_eq!(out, "event: alert_changed\ndata: {\"a\":1}\n\nevent: ui_event\ndata: {\"b\":2}\n\n");
    }

    #[tokio::test]
    async fn team_filter() {
        let items = || {
            vec![
                item("alert_changed", "1", Some(&["settlements"])),
                item("alert_changed", "2", Some(&["rates"])),
                item("alert_changed", "3", Some(&[])),
                item("ui_event", "4", None),
            ]
        };
        let all = body(items(), None).await;
        for d in ["1", "2", "3", "4"] {
            assert!(all.contains(&format!("data: {d}\n")), "bez filtra: {d}");
        }
        let rates = body(items(), Some("rates")).await;
        assert!(rates.contains("data: 2\n") && rates.contains("data: 4\n"));
        assert!(!rates.contains("data: 1\n") && !rates.contains("data: 3\n"));
    }

    #[tokio::test]
    async fn lagged_client_skips_and_continues() {
        let (tx, rx) = broadcast::channel(2);
        for i in 0..5 {
            tx.send(item("ui_event", &i.to_string(), None)).unwrap();
        }
        drop(tx);
        let bytes = axum::body::to_bytes(Sse::new(client_stream(rx, None)).into_response().into_body(), usize::MAX)
            .await
            .unwrap();
        let out = String::from_utf8(bytes.to_vec()).unwrap();
        assert_eq!(out, "event: ui_event\ndata: 3\n\nevent: ui_event\ndata: 4\n\n");
    }

    #[tokio::test]
    async fn stream_ends_on_shutdown() {
        let (_hub, rx) = broadcast::channel::<Arc<Item>>(4);
        let (stop, shutdown) = watch::channel(false);
        // Bez sygnału strumień wisi (nadawca hub żyje).
        let mut s = Box::pin(until_shutdown(client_stream(rx, None), shutdown));
        assert!(tokio::time::timeout(Duration::from_millis(50), s.next()).await.is_err());
        stop.send(true).unwrap();
        let end = tokio::time::timeout(Duration::from_secs(1), s.next()).await.expect("koniec po sygnale");
        assert!(end.is_none());
    }

    #[tokio::test]
    async fn unknown_alert_ui_event_only_for_unfiltered_clients() {
        let db = common::db::sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(Duration::from_millis(500))
            .connect_lazy("postgres://nobody@127.0.0.1:1/none")
            .unwrap();
        let mut resolver = TeamResolver::new(db);
        let mut u: UiEvent = serde_json::from_str(UI).unwrap();
        u.alert_id = Some(Uuid::new_v4());
        let teams = resolver.resolve(&Decoded::Ui(u.clone())).await;
        assert_eq!(teams, Some(Vec::new()));
        let it = Item { name: "ui_event", data: "x".into(), teams };
        assert!(!visible(&it, Some("nobody")));
        assert!(visible(&it, None));

        u.alert_id = None;
        assert_eq!(resolver.resolve(&Decoded::Ui(u)).await, None, "bez alert_id: dla wszystkich");
    }

    #[test]
    fn teams_from_matching_rules() {
        let rule: contracts::RuleDef = serde_json::from_value(serde_json::json!({
            "id": "r1", "name": "r1", "team": "settlements",
            "condition": { "field": "schema", "op": "eq", "value": "workflow.v1" },
            "recipients": [{ "type": "team", "id": "ops" }]
        }))
        .unwrap();
        let other: contracts::RuleDef = serde_json::from_value(serde_json::json!({
            "id": "r2", "name": "r2", "team": "rates",
            "condition": { "field": "schema", "op": "eq", "value": "nope" },
            "recipients": []
        }))
        .unwrap();
        let rules: Vec<CompiledRule> = [rule, other].into_iter().filter_map(CompiledRule::from_def).collect();
        let a: AlertChanged = serde_json::from_str(ALERT).unwrap();
        assert_eq!(teams_for_event(&rules, &a.event), vec!["settlements", "ops"]);
    }
}
