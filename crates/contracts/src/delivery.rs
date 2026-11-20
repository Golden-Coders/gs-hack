//! Dostarczenia: zlecenia od routera (P2) i akcji runbooków (P3) do sendera (P2).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Severity, Timestamp};

/// Kanał dostarczenia.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    Ui,
    Email,
    Sms,
    Phone,
}

/// Treść wyrenderowana dla konkretnego kanału.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rendered {
    pub title: String,
    pub body: String,
    /// Link do szczegółów alertu w UI (używany np. w SMS).
    #[serde(default)]
    pub link: Option<String>,
}

/// Skąd pochodzi zlecenie dostarczenia.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DeliveryOrigin {
    /// Dopasowana reguła (router, P2).
    Rule { rule_id: String },
    /// Akcja `escalate` / `notify` z runbooka (P3).
    Action { run_id: Uuid, step_id: String },
}

/// Status dostarczenia (tabela `deliveries`, `ui.events`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStatus {
    Pending,
    Retrying,
    Delivered,
    Fallback,
    Failed,
    Dlq,
}

/// Topiki `deliveries.requested`, `deliveries.retry`, `deliveries.dlq` (klucz = `alert_id`).
///
/// Router rozwiązuje wszystko z góry (odbiorca, kanały, łańcuch fallbacku, treść,
/// adresy), więc sender nie zna reguł ani preferencji (design D7).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeliveryRequest {
    pub delivery_id: Uuid,
    pub alert_id: Uuid,
    pub episode: i32,
    pub person_id: String,
    pub severity: Severity,
    pub origin: DeliveryOrigin,
    /// Łańcuch kanałów: najpierw podstawowy, potem zapasowe. Zawsze kończy się `Channel::Ui`.
    pub chain: Vec<Channel>,
    /// Indeks w `chain` aktualnie próbowanego kanału.
    #[serde(default)]
    pub chain_index: usize,
    /// Adres per kanał (email, numer telefonu). `Ui` nie wymaga adresu.
    pub contacts: BTreeMap<Channel, String>,
    /// Treść per kanał.
    pub rendered: BTreeMap<Channel, Rendered>,
    /// `received_at` zdarzenia źródłowego – do pomiaru latencji end-to-end.
    pub received_at: Timestamp,
    /// Numer próby na bieżącym kanale (od 0).
    #[serde(default)]
    pub attempt: u32,
    /// Nie wcześniej niż (dla `deliveries.retry`).
    #[serde(default)]
    pub not_before: Option<Timestamp>,
    /// Ostatni błąd (dla `deliveries.retry` i `deliveries.dlq`).
    #[serde(default)]
    pub last_error: Option<String>,
}

impl DeliveryRequest {
    /// Aktualnie próbowany kanał.
    pub fn current_channel(&self) -> Option<Channel> {
        self.chain.get(self.chain_index).copied()
    }
}
