//! Runbooki (definicje, P3) i komendy wykonania z UI.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::{Actor, Condition, Timestamp};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunbookKind {
    /// Kroki z akcjami z rejestru + kroki manualne/notatki, wykonywane sekwencyjnie.
    Automatic,
    /// Wolny tekst / Markdown, bez parsowania na kroki.
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepType {
    Check,
    Action,
    Manual,
    Note,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Approval {
    #[default]
    None,
    Required,
}

/// Czy krok wykona się automatycznie. `Manual`, gdy akcji nie ma w rejestrze
/// albo krok jest instrukcją dla człowieka.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Automation {
    Automated,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepDef {
    pub id: String,
    #[serde(rename = "type")]
    pub step_type: StepType,
    /// Opis dla człowieka; może zawierać `{{zmienne}}`.
    pub text: String,
    /// Nazwa akcji z rejestru (dla `check`/`action`).
    #[serde(default)]
    pub action: Option<String>,
    /// Parametry akcji; wartości mogą być szablonami.
    #[serde(default)]
    pub params: Map<String, Value>,
    #[serde(default)]
    pub approval: Approval,
    /// Warunek z runbooka w formie tekstu (nie jest ewaluowany).
    #[serde(default)]
    pub condition: Option<String>,
    pub automation: Automation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunbookDef {
    pub id: String,
    pub title: String,
    pub kind: RunbookKind,
    /// Warunek dopasowania (ten sam język co reguły).
    #[serde(default)]
    pub matches: Option<Condition>,
    /// Skrót „przypięty do reguły”: id reguł, do których alertów runbook pasuje.
    #[serde(default)]
    pub rule_ids: Vec<String>,
    /// Treść runbooka MANUAL (Markdown / tekst).
    #[serde(default)]
    pub body: Option<String>,
    /// Kroki runbooka AUTOMATIC.
    #[serde(default)]
    pub steps: Vec<StepDef>,
    /// Plik konfiguracyjny, z którego załadowano runbook.
    #[serde(default)]
    pub source_file: Option<String>,
    #[serde(default)]
    pub version: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    WaitingApproval,
    WaitingManual,
    Skipped,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunbookCommandKind {
    Approve,
    Reject,
    Done {
        #[serde(default)]
        note: Option<String>,
    },
    Retry,
    Skip,
    /// Zatrzymaj automatykę; pozostałe kroki stają się manualne.
    ManualMode,
}

/// Topik `runbooks.commands` (klucz = `alert_id`). Producent: api (moduł P3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunbookCommand {
    pub alert_id: Uuid,
    pub run_id: Uuid,
    pub step_id: String,
    pub kind: RunbookCommandKind,
    pub actor: Actor,
    pub at: Timestamp,
}
