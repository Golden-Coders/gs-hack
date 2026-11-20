//! lifecycle – normalizer (events.raw -> events.normalized) + dedup i stan alertów (właściciel: P1).
//!
//! `mapping` jest czystą logiką (bez IO) i bywa używany jako biblioteka (np. podgląd mappingu w api).

pub mod config;
pub mod dedup;
pub mod mapping;
pub mod normalizer;

pub use config::{load_mappings, run_config, MappingStore};
pub use dedup::{run_dedup, run_dedup_with};
pub use normalizer::{replay_unmapped, run_normalizer, run_normalizer_with};

/// Sygnał zatrzymania dla tasków (`true` = zatrzymaj).
pub type Shutdown = tokio::sync::watch::Receiver<bool>;
