//! runbooks – dopasowanie i wykonanie runbooków, rejestr akcji, zatwierdzenia (właściciel: P3).
//!
//! - `parse`, `store` – runbooki i akcje z `config/` → Postgres → migawka (hot reload),
//! - `registry` – rejestr akcji i bezpieczne wykonanie HTTP,
//! - `matching` – dopasowanie runbooków do alertu, podstawianie wartości,
//! - `engine` – runy raz na epizod, maszyna stanów kroków, komendy z UI,
//! - `author` – `RunbookAuthor` (w MVP tylko `none`).

mod author;
mod engine;
#[cfg(test)]
mod it;
mod matching;
mod notify;
mod parse;
mod registry;
mod store;

use std::sync::Arc;

use arc_swap::ArcSwap;

/// Domyślny maks. rozmiar puli Postgresa (nadpisywany przez `DB_MAX_CONNECTIONS`, zob. docs/LOADTEST.md).
const DB_POOL_DEFAULT: u32 = 6;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    common::init_tracing("runbooks");
    let config = common::Config::from_env();
    let pool = common::db::connect_and_migrate_with(&config.database_url, DB_POOL_DEFAULT).await?;

    // Pierwsze wczytanie przed konsumentem: bez runbooków alert przeszedłby bez runów.
    let author = author::from_env()?;
    tracing::info!(provider = author.name(), "runbook author");

    let producer = common::kafka::producer(&config.kafka_brokers)?;
    let snapshot = Arc::new(ArcSwap::from_pointee(store::Snapshot::default()));
    store::reload_once(&pool, &config.config_dir, &snapshot).await?;
    tokio::spawn(store::run(pool.clone(), config.config_dir.clone(), snapshot.clone(), producer.clone()));

    let engine = Arc::new(engine::Engine {
        pool,
        producer,
        executor: registry::Executor::new(config.action_host_allowlist.clone()),
        snapshot,
        public_ui_url: config.public_ui_url.clone(),
    });
    engine.resume_all().await?;
    let consumer = tokio::spawn(engine.clone().consume(config.kafka_brokers.clone()));

    tracing::info!("runbooks ready");
    tokio::select! {
        _ = common::shutdown_signal() => {}
        result = consumer => result??,
    }
    Ok(())
}

/// Seed z `config/` zgadza się z fixtures kontraktu (to samo widzą P2 i P4).
#[cfg(test)]
mod seed_tests {
    use contracts::{ActionDef, RunbookDef};

    use crate::{parse, registry};

    const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

    fn read(rel: &str) -> String {
        std::fs::read_to_string(format!("{ROOT}/{rel}")).unwrap_or_else(|e| panic!("{rel}: {e}"))
    }

    fn fixture<T: serde::de::DeserializeOwned>(name: &str) -> T {
        serde_json::from_str(&read(&format!("crates/contracts/fixtures/config/{name}"))).unwrap()
    }

    fn seed_actions() -> Vec<ActionDef> {
        let dir = format!("{ROOT}/config/actions");
        let mut actions = Vec::new();
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "yaml") {
                let file = path.display().to_string();
                let action = parse::parse_action(&file, &std::fs::read_to_string(&path).unwrap()).unwrap();
                registry::validate_action(&action).unwrap_or_else(|e| panic!("{file}: {e}"));
                actions.push(action);
            }
        }
        actions
    }

    #[test]
    fn seed_actions_are_valid_and_flush_cache_matches_fixture() {
        let actions = seed_actions();
        let names: Vec<_> = actions.iter().map(|a| a.name.as_str()).collect();
        for expected in ["retry_workflow", "restart_service", "open_ticket", "flush_cache"] {
            assert!(names.contains(&expected), "missing seed action {expected}");
        }
        let flush = actions.into_iter().find(|a| a.name == "flush_cache").unwrap();
        let expected: ActionDef = fixture("action.flush_cache.json");
        assert_eq!(ActionDef { version: expected.version, ..flush }, expected);
    }

    #[test]
    fn settlement_timeout_matches_fixture() {
        let file = "config/runbooks/settlement-timeout.yaml";
        let def = parse::parse_runbook(file, &read(file)).unwrap();
        let resolved = registry::Registry::new(seed_actions()).classify(&def);
        let expected: RunbookDef = fixture("runbook.settlement_timeout.json");
        assert_eq!(resolved.steps, expected.steps);
        assert_eq!((&resolved.id, &resolved.title, resolved.kind), (&expected.id, &expected.title, expected.kind));
        assert_eq!(resolved.source_file, expected.source_file);
        // `matches` ze skrótu: te same warunki, kolejność kluczy alfabetyczna.
        let (Some(contracts::Condition::All { all: got }), Some(contracts::Condition::All { all: want })) = (&resolved.matches, &expected.matches) else {
            panic!("matches")
        };
        assert_eq!(got.len(), want.len());
        assert!(want.iter().all(|c| got.contains(c)));
    }

    #[test]
    fn payments_manual_matches_fixture() {
        let file = "config/runbooks/payments-high-error-rate.md";
        let def = parse::parse_runbook(file, &read(file)).unwrap();
        let expected: RunbookDef = fixture("runbook.payments_manual.json");
        assert_eq!(RunbookDef { version: expected.version, ..def }, expected);
    }

    #[test]
    fn pricing_free_text_loads() {
        let file = "config/runbooks/pricing-latency.txt";
        let def = parse::parse_runbook(file, &read(file)).unwrap();
        assert_eq!(def.kind, contracts::RunbookKind::Manual);
        assert!(def.matches.is_some());
        assert!(def.body.unwrap().contains("usually the cache"));
    }
}
