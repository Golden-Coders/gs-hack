//! Alerty: zmiany stanu publikowane przez lifecycle (P1) i komendy z UI.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Actor, CanonicalEvent, Severity, Timestamp};

/// Stan alertu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertState {
    Open,
    Acked,
    Resolved,
    Reopened,
}

/// Przejście stanu alertu.
///
/// Zmiany samego licznika (deduplikacja) NIE są publikowane na `alerts.changed`
/// – idą na `ui.events` jako `UiEventKind::AlertCount` (design D4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transition {
    Opened,
    Reopened,
    Acked,
    Resolved,
}

/// Topik `alerts.changed` (klucz = `alert_id`). Jedynym producentem jest lifecycle (P1).
/// Konsumenci: router (P2), runbooks (P3), api (SSE).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlertChanged {
    pub alert_id: Uuid,
    pub fingerprint: String,
    /// Numer epizodu, od 1. Rośnie przy każdym ponownym otwarciu.
    pub episode: i32,
    pub transition: Transition,
    pub state: AlertState,
    pub severity: Severity,
    pub count: i64,
    /// Zdarzenie, które spowodowało przejście (dla ack/resolve z UI – ostatnie znane zdarzenie).
    pub event: CanonicalEvent,
    /// Kto spowodował przejście (`None` dla otwarcia przez zdarzenie).
    #[serde(default)]
    pub actor: Option<Actor>,
    pub at: Timestamp,
}

/// Rodzaj komendy dla alertu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertCommandKind {
    Ack,
    Resolve,
}

/// Topik `alerts.commands` (klucz = `fingerprint`, żeby trafić do instancji
/// lifecycle, która jest jedynym pisarzem alertu). Producent: api (moduł P1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlertCommand {
    pub alert_id: Uuid,
    pub fingerprint: String,
    pub kind: AlertCommandKind,
    pub actor: Actor,
    pub at: Timestamp,
}
