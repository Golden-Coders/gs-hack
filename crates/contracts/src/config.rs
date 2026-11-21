//! Definicje konfiguracji współdzielone między strumieniami i UI:
//! mappingi (P1), reguły i osoby (P2), akcje (P3). Runbooki: `runbook.rs`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Channel, Condition, Severity};

/// Jak wyznaczyć severity w mappingu.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SeveritySpec {
    /// Wartość z pola + słownik nazw źródła na wspólną skalę (np. `ERROR -> error`).
    Path {
        path: String,
        #[serde(default)]
        map: BTreeMap<String, Severity>,
        #[serde(default = "default_severity")]
        default: Severity,
    },
    /// Stała wartość.
    Const { value: Severity },
    /// Pierwszy spełniony warunek wygrywa; inaczej `default`.
    Rules {
        rules: Vec<SeverityRule>,
        #[serde(default = "default_severity")]
        default: Severity,
    },
}

fn default_severity() -> Severity {
    Severity::Warning
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeverityRule {
    pub when: Condition,
    pub value: Severity,
}

/// Mapping schematu na zdarzenie kanoniczne (design D2/D4, spec `event-normalization`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MappingDef {
    pub schema: String,
    /// Nazwa pola -> ścieżka JSONPath w payloadzie, np. `service: $.labels.service`.
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
    /// Ścieżka JSONPath albo szablon `{{...}}` dla tytułu.
    #[serde(default)]
    pub title: Option<String>,
    pub severity: SeveritySpec,
    /// Ścieżki JSONPath tworzące fingerprint. Pusta lista = schema + cały payload.
    #[serde(default)]
    pub fingerprint: Vec<String>,
    /// Warunek oznaczający zdarzenie jako rozwiązujące problem.
    #[serde(default)]
    pub resolved: Option<Condition>,
    #[serde(default)]
    pub version: i64,
}

/// Odbiorca wskazany przez regułę.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum Recipient {
    Person(String),
    Team(String),
}

/// Prezentacja alertu dla reguły.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Presentation {
    /// Szablon tytułu, np. `Rejected {{raw.payload.currency}} {{raw.payload.notional}}`.
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    /// Pola wyświetlane w UI (ścieżki jak w warunkach).
    #[serde(default)]
    pub fields: Vec<String>,
    /// Pola wyróżnione kolorem, np. `{ "raw.step": "red" }`.
    #[serde(default)]
    pub highlight: BTreeMap<String, String>,
    /// Kolor alertu w UI.
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleDef {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub team: Option<String>,
    pub condition: Condition,
    pub recipients: Vec<Recipient>,
    #[serde(default)]
    pub severity_override: Option<Severity>,
    #[serde(default)]
    pub presentation: Presentation,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub version: i64,
}

fn default_true() -> bool {
    true
}

/// Preferencja: dla severity (i opcjonalnie przedziału godzin) -> kanały.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreferenceDef {
    pub severity: Severity,
    pub channels: Vec<Channel>,
    /// Przedział godzin w strefie osoby, np. `"08:00-20:00"`. Brak = całą dobę.
    #[serde(default)]
    pub hours: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersonDef {
    pub id: String,
    pub name: String,
    /// Strefa IANA, np. `Europe/Warsaw`.
    #[serde(default = "default_tz")]
    pub timezone: String,
    #[serde(default)]
    pub contacts: BTreeMap<Channel, String>,
    #[serde(default)]
    pub preferences: Vec<PreferenceDef>,
    /// Kanały zapasowe w kolejności; UI jest zawsze dokładany na końcu.
    #[serde(default)]
    pub fallback: Vec<Channel>,
}

fn default_tz() -> String {
    "Europe/Warsaw".to_string()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TeamDef {
    pub id: String,
    pub name: String,
    pub members: Vec<String>,
    /// Domyślne preferencje dla członków bez własnych.
    #[serde(default)]
    pub preferences: Vec<PreferenceDef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    Low,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    /// Akcja zaimplementowana w kodzie (`escalate`, `notify`, `http_get`).
    Builtin,
    /// Akcja deklaratywna HTTP (YAML / formularz UI).
    Http,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HttpActionSpec {
    pub method: String,
    /// Szablon URL, np. `http://cache-admin/{{service}}/flush`.
    pub url: String,
    /// Wartości mogą zawierać referencje do sekretów `${ENV_NAME}`.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_timeout_ms() -> u64 {
    5000
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionDef {
    pub name: String,
    pub kind: ActionKind,
    pub risk: Risk,
    #[serde(default)]
    pub description: Option<String>,
    /// Opcjonalny JSON Schema parametrów kroku.
    #[serde(default)]
    pub params_schema: Option<Value>,
    #[serde(default)]
    pub http: Option<HttpActionSpec>,
    #[serde(default)]
    pub version: i64,
}
