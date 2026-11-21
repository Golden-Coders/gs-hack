//! lifecycle – normalizer (events.raw -> events.normalized) + dedup i stan alertów (właściciel: P1).
//!
//! `mapping` jest czystą logiką (bez IO) i bywa używany jako biblioteka (np. podgląd mappingu w api).

pub mod config;
pub mod dedup;
pub mod mapping;
pub mod normalizer;

pub use config::{load_mappings, run_config, MappingStore};
pub use dedup::{run_dedup, run_dedup_with};
pub use normalizer::{replay_unmapped, replay_unmapped_to, run_normalizer, run_normalizer_with};

/// Sygnał zatrzymania dla tasków (`true` = zatrzymaj).
pub type Shutdown = tokio::sync::watch::Receiver<bool>;

/// Topiki używane przez role lifecycle. Produkcyjnie stałe z `contracts::topics` (`Default`);
/// testy biorą `with_prefix`, żeby nie dzielić topików z instancją działającą w compose.
#[derive(Debug, Clone)]
pub struct Topics {
    pub events_raw: String,
    pub events_unmapped: String,
    pub events_normalized: String,
    pub alerts_commands: String,
    pub alerts_changed: String,
    pub ui_events: String,
}

impl Topics {
    pub fn with_prefix(prefix: &str) -> Self {
        use contracts::topics::*;
        let t = |name: &str| if prefix.is_empty() { name.to_string() } else { format!("{prefix}.{name}") };
        Self {
            events_raw: t(EVENTS_RAW),
            events_unmapped: t(EVENTS_UNMAPPED),
            events_normalized: t(EVENTS_NORMALIZED),
            alerts_commands: t(ALERTS_COMMANDS),
            alerts_changed: t(ALERTS_CHANGED),
            ui_events: t(UI_EVENTS),
        }
    }
}

impl Default for Topics {
    fn default() -> Self {
        Self::with_prefix("")
    }
}
