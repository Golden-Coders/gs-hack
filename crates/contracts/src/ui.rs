//! Zdarzenia dla UI (topik `ui.events`, rozsyłane przez api przez SSE).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::Timestamp;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiEventKind {
    /// P1: nowy licznik / last_seen alertu. payload: `{count, last_seen}`.
    AlertCount,
    /// P1: nowe lub zmienione zdarzenie UNMAPPED. payload: `{schema, count}`.
    Unmapped,
    /// P1: błąd ingestu. payload: `{source, error}`.
    IngestError,
    /// P2: zmiana statusu dostarczenia. payload: wiersz dostarczenia.
    DeliveryStatus,
    /// P2: nowa wiadomość w symulowanym kanale. payload: wiersz `mock_outbox`.
    MockOutbox,
    /// P3: zmiana statusu kroku runbooka. payload: `{run_id, step_id, status, ...}`.
    StepStatus,
    /// P3: nowy wpis w dzienniku działań. payload: wpis.
    AuditEntry,
    /// P1/P2/P3: przeładowano konfigurację. payload: `{kind}`.
    ConfigChanged,
}

/// Topik `ui.events` (klucz = `alert_id` albo rodzaj zdarzenia, gdy brak alertu).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiEvent {
    #[serde(default)]
    pub alert_id: Option<Uuid>,
    pub kind: UiEventKind,
    pub payload: Value,
    pub at: Timestamp,
}
