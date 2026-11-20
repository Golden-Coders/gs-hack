//! Zdarzenia wejściowe: koperta i zdarzenie kanoniczne.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::Timestamp;

/// Znane wartości pola `format`. Pole jest stringiem, żeby nowy format
/// wymagał tylko nowego parsera, a nie zmiany kontraktu.
pub mod formats {
    pub const JSON: &str = "json";
    pub const YAML: &str = "yaml";
}

/// Koperta przyjmowana od klienta na `POST /ingest` (przed nadaniem identyfikatora).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IncomingEnvelope {
    pub format: String,
    pub schema: String,
    /// Dla `json` – dowolna wartość JSON; dla formatów tekstowych (np. `yaml`) – string.
    pub payload: Value,
}

/// Koperta po przyjęciu przez ingest. Topiki: `events.raw`, `events.unmapped`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub event_id: Uuid,
    pub format: String,
    pub schema: String,
    pub payload: Value,
    /// Skąd przyszło zdarzenie, np. `http`, `http:alertmanager`, `kafka:trades`.
    pub source: String,
    /// Moment przyjęcia przez ingest – początek pomiaru latencji end-to-end.
    pub received_at: Timestamp,
}

/// Wspólna skala severity. Kolejność wariantów = kolejność ważności (rosnąco),
/// więc `Severity::Critical > Severity::Warning`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
    Critical,
}

/// Zdarzenie po normalizacji. Topik: `events.normalized` (klucz = `fingerprint`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanonicalEvent {
    pub event_id: Uuid,
    pub schema: String,
    pub source: String,
    pub severity: Severity,
    pub title: String,
    pub fingerprint: String,
    /// `true`, gdy zdarzenie spełnia warunek `resolved` z mappingu.
    pub resolved: bool,
    /// Pola wyciągnięte przez mapping (nazwa pola kanonicznego/dodatkowego -> wartość).
    pub fields: Map<String, Value>,
    /// Oryginalny payload (po sparsowaniu formatu). W regułach dostępny jako `raw.*`.
    pub raw: Value,
    pub received_at: Timestamp,
    pub mapping_version: i64,
}
