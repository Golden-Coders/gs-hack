//! Nazwy topików Kafki i ich klucze (design D2).
//!
//! | Topik                  | Klucz        | Producent                 | Konsument                      |
//! |------------------------|--------------|---------------------------|--------------------------------|
//! | events.raw             | source       | ingest (P1)               | lifecycle-normalizer (P1)      |
//! | events.unmapped        | schema       | lifecycle (P1)            | lifecycle-replay (P1)          |
//! | events.normalized      | fingerprint  | lifecycle-normalizer (P1) | lifecycle-dedup (P1)           |
//! | alerts.commands        | fingerprint  | api/p1                    | lifecycle-dedup (P1)           |
//! | alerts.changed         | alert_id     | lifecycle (P1)            | router, runbooks, api          |
//! | deliveries.requested   | alert_id     | router (P2), runbooks (P3)| sender (P2)                    |
//! | deliveries.retry       | alert_id     | sender (P2)               | sender-retry (P2)              |
//! | deliveries.dlq         | alert_id     | sender (P2)               | api                            |
//! | runbooks.commands      | alert_id     | api/p3                    | runbooks (P3)                  |
//! | ui.events              | alert_id     | P1, P2, P3                | api (SSE)                      |

pub const EVENTS_RAW: &str = "events.raw";
pub const EVENTS_UNMAPPED: &str = "events.unmapped";
pub const EVENTS_NORMALIZED: &str = "events.normalized";
pub const ALERTS_COMMANDS: &str = "alerts.commands";
pub const ALERTS_CHANGED: &str = "alerts.changed";
pub const DELIVERIES_REQUESTED: &str = "deliveries.requested";
pub const DELIVERIES_RETRY: &str = "deliveries.retry";
pub const DELIVERIES_DLQ: &str = "deliveries.dlq";
pub const RUNBOOKS_COMMANDS: &str = "runbooks.commands";
pub const UI_EVENTS: &str = "ui.events";

/// Wszystkie topiki wewnętrzne (tworzone przy starcie compose).
pub const ALL: [&str; 10] = [
    EVENTS_RAW,
    EVENTS_UNMAPPED,
    EVENTS_NORMALIZED,
    ALERTS_COMMANDS,
    ALERTS_CHANGED,
    DELIVERIES_REQUESTED,
    DELIVERIES_RETRY,
    DELIVERIES_DLQ,
    RUNBOOKS_COMMANDS,
    UI_EVENTS,
];

/// Liczba partycji każdego topiku wewnętrznego = maks. liczba instancji etapu.
pub const PARTITIONS: i32 = 12;

/// Grupy konsumentów (każda rola ma własną).
pub mod groups {
    pub const LIFECYCLE_NORMALIZER: &str = "lifecycle-normalizer";
    pub const LIFECYCLE_REPLAY: &str = "lifecycle-replay";
    pub const LIFECYCLE_DEDUP: &str = "lifecycle-dedup";
    pub const ROUTER: &str = "router";
    pub const RUNBOOKS: &str = "runbooks";
    pub const SENDER: &str = "sender";
    pub const SENDER_RETRY: &str = "sender-retry";
    /// Prefiks; każda instancja api dodaje własny sufiks, żeby dostać wszystkie zdarzenia.
    pub const API_SSE_PREFIX: &str = "api-sse";
}
