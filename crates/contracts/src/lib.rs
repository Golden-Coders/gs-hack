//! Kontrakty między strumieniami P1–P4 (Faza 0).
//!
//! ZAMROŻONE: dozwolone są wyłącznie zmiany addytywne (nowe pole jako `Option`
//! z `#[serde(default)]`, nowy wariant na końcu enuma). Zmiana łamiąca wymaga
//! zgody całego zespołu. Zob. `openspec/changes/alerting-service-mvp/team.md`.

pub mod alert;
pub mod condition;
pub mod config;
pub mod delivery;
pub mod event;
pub mod keys;
pub mod runbook;
pub mod topics;
pub mod ui;

pub use alert::*;
pub use condition::*;
pub use config::*;
pub use delivery::*;
pub use event::*;
pub use runbook::*;
pub use ui::*;

/// Znacznik czasu używany we wszystkich kontraktach (UTC).
pub type Timestamp = chrono::DateTime<chrono::Utc>;

/// Kto wykonał działanie. Zapisywany w dzienniku i przy przejściach stanu.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Actor {
    /// Człowiek; `person_id` z konfiguracji osób (wybór tożsamości w UI).
    User { person_id: String },
    /// Sam system (np. automatyczny krok runbooka).
    System,
    /// Źródło zdarzeń (np. zdarzenie `resolved` z mappingu).
    Source,
}
