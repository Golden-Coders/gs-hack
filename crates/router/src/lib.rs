//! router – ewaluacja reguł, rozwiązywanie odbiorców i preferencji kanałów/godzin,
//! dołączanie runbooków MANUAL oraz publikacja `deliveries.requested` (właściciel: P2).

pub mod config;
pub mod engine;

use std::time::Duration;

use common::kafka::rdkafka::consumer::{CommitMode, Consumer};
use common::kafka::rdkafka::message::BorrowedMessage;
use common::kafka::rdkafka::producer::FutureProducer;
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

/// Jak `run_router`, ale z podaną grupą konsumentów (testy integracyjne mogą używać własnej grupy).
pub async fn run_router_with(
    group: &str,
    brokers: String,
    public_ui_url: String,
    producer: FutureProducer,
    store: RouterStore,
    mut shutdown: Shutdown,
) -> anyhow::Result<()> {
    let consumer = common::kafka::consumer(&brokers, group, &[topics::ALERTS_CHANGED])?;
    loop {
        let msg = tokio::select! {
            _ = shutdown.changed() => return Ok(()),
            m = consumer.recv() => match m {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(error = %e, "router: kafka recv error");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            },
        };

        // At-least-once: ponawiamy publikację wszystkich wyznaczonych dostarczeń,
        // a offset zatwierdzamy DOPIERO po opublikowaniu kompletu zleceń (design D2, D7).
        loop {
            match handle_message(&msg, &producer, &store, &public_ui_url).await {
                Ok(()) => break,
                Err(e) => {
                    tracing::warn!(error = %e, "router: processing alert failed, retrying");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }

        if let Err(e) = consumer.commit_message(&msg, CommitMode::Async) {
            tracing::warn!(error = %e, "router: offset commit failed");
        }
    }
}

async fn handle_message(
    msg: &BorrowedMessage<'_>,
    producer: &FutureProducer,
    store: &RouterStore,
    public_ui_url: &str,
) -> anyhow::Result<()> {
    let alert: AlertChanged = match common::kafka::decode(msg) {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!(error = %e, "router: undecodable AlertChanged message, skipped");
            return Ok(());
        }
    };

    let snapshot = store.load_full();
    let requests = route_alert(&snapshot, &alert, public_ui_url);
    let key = alert.alert_id.to_string();

    for req in &requests {
        common::kafka::publish(producer, topics::DELIVERIES_REQUESTED, &key, req).await?;
    }

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
