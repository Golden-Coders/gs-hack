//! Typy języka warunków (design D6). Implementacja ewaluacji: crate `condition` (P2).
//!
//! Format (JSON/YAML):
//! ```yaml
//! all:
//!   - { field: schema, op: glob, value: "trade.rejected.*" }
//!   - any:
//!       - { field: raw.error.code, op: eq, value: TIMEOUT }
//!       - { field: raw.error.code, op: eq, value: REJECTED }
//!   - not: { field: environment, op: eq, value: dev }
//! ```

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Operator porównania w liściu warunku.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    Eq,
    Ne,
    In,
    Gt,
    Gte,
    Lt,
    Lte,
    Contains,
    Regex,
    Exists,
    Glob,
}

/// Drzewo warunków.
///
/// `field` to ścieżka po polach kanonicznych (`schema`, `severity`, `source`, `title`,
/// pola z `fields`) albo po oryginale z prefiksem `raw.` (np. `raw.payload.notional`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Condition {
    All {
        all: Vec<Condition>,
    },
    Any {
        any: Vec<Condition>,
    },
    Not {
        not: Box<Condition>,
    },
    Leaf {
        field: String,
        op: Op,
        /// Pomijane dla `exists`.
        #[serde(default)]
        value: Value,
    },
}

/// Wynik `explain`: drzewo z wynikiem każdego węzła (do „testuj na zdarzeniu”).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Explanation {
    pub result: bool,
    /// Opis węzła, np. `raw.payload.notional gt 10000000` albo `all`.
    pub node: String,
    /// Faktyczna wartość pola (tylko dla liści).
    #[serde(default)]
    pub actual: Option<Value>,
    #[serde(default)]
    pub children: Vec<Explanation>,
}
