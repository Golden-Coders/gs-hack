//! Ewaluator warunków i render szablonów (design D6). Właściciel: P2.
//!
//! PUBLICZNE API JEST ZAMROŻONE (Faza 0) – używają go `lifecycle` (P1), `router` (P2)
//! i `runbooks` (P3). P2 wymienia implementację, nie sygnatury.
//!
//! STAN: ZAŚLEPKA. `evaluate` zawsze zwraca `false`, `render_template` zwraca
//! szablon bez zmian. Prawdziwa implementacja: `tasks.md` sekcja 6.

use contracts::{CanonicalEvent, Condition, Explanation};
use serde_json::{json, Value};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid condition: {0}")]
    InvalidCondition(String),
    #[error("invalid template: {0}")]
    InvalidTemplate(String),
}

/// Kontekst ewaluacji: pola kanoniczne na najwyższym poziomie + oryginał pod `raw`.
///
/// Ścieżka `severity` czyta pole kanoniczne, `raw.payload.notional` – oryginał,
/// a nazwy z `fields` (np. `service`) są dostępne bezpośrednio.
pub fn event_context(event: &CanonicalEvent) -> Value {
    let mut ctx = serde_json::Map::new();
    for (k, v) in &event.fields {
        ctx.insert(k.clone(), v.clone());
    }
    ctx.insert("event_id".into(), json!(event.event_id));
    ctx.insert("schema".into(), json!(event.schema));
    ctx.insert("source".into(), json!(event.source));
    ctx.insert("severity".into(), json!(event.severity));
    ctx.insert("title".into(), json!(event.title));
    ctx.insert("fingerprint".into(), json!(event.fingerprint));
    ctx.insert("resolved".into(), json!(event.resolved));
    ctx.insert("received_at".into(), json!(event.received_at));
    ctx.insert("raw".into(), event.raw.clone());
    Value::Object(ctx)
}

/// Warunek po kompilacji (regexy, globy prekompilowane). Tanie do współdzielenia.
#[derive(Debug, Clone)]
pub struct Compiled {
    source: Condition,
}

/// Kompiluje warunek. Błąd = niepoprawny warunek (np. zły regex, `in` bez tablicy).
pub fn compile(condition: &Condition) -> Result<Compiled, Error> {
    Ok(Compiled { source: condition.clone() })
}

impl Compiled {
    /// Czy kontekst (zob. [`event_context`]) spełnia warunek.
    pub fn evaluate(&self, _ctx: &Value) -> bool {
        // ZAŚLEPKA
        false
    }

    /// Wynik każdego węzła drzewa – do „testuj na zdarzeniu”.
    pub fn explain(&self, _ctx: &Value) -> Explanation {
        // ZAŚLEPKA
        Explanation { result: false, node: "stub".into(), actual: None, children: vec![] }
    }

    /// Warunek źródłowy.
    pub fn source(&self) -> &Condition {
        &self.source
    }
}

/// Sprawdza szablon przy zapisie (składnia, brak niedozwolonych konstrukcji).
pub fn validate_template(_template: &str) -> Result<(), Error> {
    // ZAŚLEPKA
    Ok(())
}

/// Renderuje szablon `{{ścieżka}}` w sandboxie. Brakująca zmienna = pusty tekst.
pub fn render_template(template: &str, _ctx: &Value) -> Result<String, Error> {
    // ZAŚLEPKA
    Ok(template.to_string())
}
