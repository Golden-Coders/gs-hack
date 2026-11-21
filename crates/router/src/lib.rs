//! router – ewaluacja reguł, rozwiązywanie odbiorców i preferencji kanałów/godzin,
//! dołączanie runbooków MANUAL oraz publikacja `deliveries.requested` (właściciel: P2).

pub mod config;
pub mod engine;

use std::sync::Arc;

use common::kafka::rdkafka::message::{Message, OwnedMessage};
use common::kafka::rdkafka::producer::FutureProducer;
use common::kafka::{concurrent_consumer, run_concurrent, ConcurrentConfig, LaneHandler};
use contracts::{topics, AlertChanged};

pub use config::{
    default_fallback_chain, load_snapshot, new_store, parse_and_validate_rule_yaml, run_config,
    scan_config_files, seed_rules, sync_people_config, sync_people_yaml, CompiledManualRunbook,
    CompiledRule, PeopleConfigFile, RouterSnapshot, RouterStore,
};
pub use engine::{
    is_within_hours, match_manual_runbooks, render_for_channels, resolve_person_channels,
    route_alert,
};

/// Sygnał zatrzymania dla tasków (`true` = zatrzymaj).
pub type Shutdown = tokio::sync::watch::Receiver<bool>;

/// Konsument `alerts.changed` w grupie `router`; działa do sygnału zatrzymania.
pub async fn run_router(
    brokers: String,
    public_ui_url: String,
    producer: FutureProducer,
    store: RouterStore,
    shutdown: Shutdown,
) -> anyhow::Result<()> {
    run_router_with(
        topics::groups::ROUTER,
        brokers,
        public_ui_url,
        producer,
        store,
        shutdown,
    )
    .await
}

/// Domyślna liczba pasów współbieżnego konsumenta (nadpisywana przez `ROUTER_LANES`).
const DEFAULT_LANES: usize = 32;

/// Jak `run_router`, ale z podaną grupą konsumentów (testy integracyjne mogą używać własnej grupy).
pub async fn run_router_with(
    group: &str,
    brokers: String,
    public_ui_url: String,
    producer: FutureProducer,
    store: RouterStore,
    shutdown: Shutdown,
) -> anyhow::Result<()> {
    run_router_on_topics(
        group,
        &brokers,
        topics::ALERTS_CHANGED,
        topics::DELIVERIES_REQUESTED,
        public_ui_url,
        producer,
        store,
        ConcurrentConfig::from_env("ROUTER", DEFAULT_LANES),
        shutdown,
    )
    .await
}

/// Jak `run_router_with`, ale z jawnymi topikami wejścia/wyjścia i konfiguracją współbieżności
/// (izolowane testy integracyjne; produkcyjnie topiki są stałe).
#[allow(clippy::too_many_arguments)]
pub async fn run_router_on_topics(
    group: &str,
    brokers: &str,
    input_topic: &str,
    output_topic: &str,
    public_ui_url: String,
    producer: FutureProducer,
    store: RouterStore,
    config: ConcurrentConfig,
    shutdown: Shutdown,
) -> anyhow::Result<()> {
    let consumer = concurrent_consumer(brokers, group, &[input_topic])?;
    let handler = RouterHandler {
        producer,
        store,
        public_ui_url: Arc::from(public_ui_url),
        output_topic: Arc::from(output_topic),
    };
    run_concurrent(consumer, config, shutdown, move |_| handler.clone()).await
}

/// Bezstanowy handler `alerts.changed` (stan = migawka reguł w `RouterStore`).
#[derive(Clone)]
struct RouterHandler {
    producer: FutureProducer,
    store: RouterStore,
    public_ui_url: Arc<str>,
    output_topic: Arc<str>,
}

impl LaneHandler for RouterHandler {
    async fn handle(&mut self, msg: &OwnedMessage) -> anyhow::Result<()> {
        let alert: AlertChanged = match serde_json::from_slice(msg.payload().unwrap_or_default()) {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!(error = %e, "router: undecodable AlertChanged message, skipped");
                return Ok(());
            }
        };

        let snapshot = self.store.load_full();
        let requests = route_alert(&snapshot, &alert, &self.public_ui_url);
        let key = alert.alert_id.to_string();

        futures::future::try_join_all(
            requests
                .iter()
                .map(|req| common::kafka::publish(&self.producer, &self.output_topic, &key, req)),
        )
        .await?;

        if !requests.is_empty() {
            tracing::info!(
                alert_id = %alert.alert_id,
                episode = alert.episode,
                transition = ?alert.transition,
                deliveries = requests.len(),
                "routed alert to deliveries.requested"
            );
        }

        Ok(())
    }
}
