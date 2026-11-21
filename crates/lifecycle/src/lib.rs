//! lifecycle – normalizer (events.raw -> events.normalized) + dedup i stan alertów (właściciel: P1).
//!
//! `mapping` jest czystą logiką (bez IO) i bywa używany jako biblioteka (np. podgląd mappingu w api).

pub mod config;
pub mod dedup;
pub mod mapping;
pub mod normalizer;

pub use config::{load_mappings, run_config, MappingStore};
pub use dedup::{run_dedup, run_dedup_with};
pub use normalizer::{replay_unmapped, replay_unmapped_to, run_normalizer, run_normalizer_with};

/// Sygnał zatrzymania dla tasków (`true` = zatrzymaj).
pub type Shutdown = tokio::sync::watch::Receiver<bool>;

/// Topiki używane przez role lifecycle. Produkcyjnie stałe z `contracts::topics` (`Default`);
/// testy biorą `with_prefix`, żeby nie dzielić topików z instancją działającą w compose.
#[derive(Debug, Clone)]
pub struct Topics {
    pub events_raw: String,
    pub events_unmapped: String,
    pub events_normalized: String,
    pub alerts_commands: String,
    pub alerts_changed: String,
    pub ui_events: String,
}

impl Topics {
    pub fn with_prefix(prefix: &str) -> Self {
        use contracts::topics::*;
        let t = |name: &str| {
            if prefix.is_empty() {
                name.to_string()
            } else {
                format!("{prefix}.{name}")
            }
        };
        Self {
            events_raw: t(EVENTS_RAW),
            events_unmapped: t(EVENTS_UNMAPPED),
            events_normalized: t(EVENTS_NORMALIZED),
            alerts_commands: t(ALERTS_COMMANDS),
            alerts_changed: t(ALERTS_CHANGED),
            ui_events: t(UI_EVENTS),
        }
    }
}

impl Default for Topics {
    fn default() -> Self {
        Self::with_prefix("")
    }
}

/// Maksymalna liczba ponowień dla trwałych błędów danych/indeksu Postgresa przed pominięciem wiadomości.
pub(crate) const MAX_PERMANENT_RETRIES: u32 = 5;

/// Rozpoznaje trwałe kody błędów Postgresa (klasa `22*` Data Exception oraz `54000` Program Limit Exceeded).
pub(crate) fn is_permanent_sqlstate(code: &str) -> bool {
    code.starts_with("22") || code == "54000"
}

/// Sprawdza, czy łańcuch błędów zawiera trwały błąd danych/indeksu z Postgresa.
pub(crate) fn is_permanent_pg_error(err: &anyhow::Error) -> bool {
    for cause in err.chain() {
        if let Some(sqlx_err) = cause.downcast_ref::<common::db::sqlx::Error>() {
            if let Some(db_err) = sqlx_err.as_database_error() {
                if let Some(code) = db_err.code() {
                    if is_permanent_sqlstate(&code) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Opakowuje handler pasa: po `MAX_PERMANENT_RETRIES` trwałych błędach Postgresa dla tej samej
/// wiadomości pomija ją (poison pill), zamiast blokować pas w nieskończoność.
pub(crate) struct Guarded<H> {
    inner: H,
    label: &'static str,
    current: Option<(i32, i64)>,
    failures: u32,
}

impl<H> Guarded<H> {
    pub(crate) fn new(inner: H, label: &'static str) -> Self {
        Self {
            inner,
            label,
            current: None,
            failures: 0,
        }
    }
}

impl<H: Clone> Clone for Guarded<H> {
    fn clone(&self) -> Self {
        Self::new(self.inner.clone(), self.label)
    }
}

impl<H: common::kafka::LaneHandler> common::kafka::LaneHandler for Guarded<H> {
    async fn handle(
        &mut self,
        msg: &common::kafka::rdkafka::message::OwnedMessage,
    ) -> anyhow::Result<()> {
        use common::kafka::rdkafka::message::Message;
        let id = (msg.partition(), msg.offset());
        if self.current != Some(id) {
            self.current = Some(id);
            self.failures = 0;
        }
        match self.inner.handle(msg).await {
            Ok(()) => {
                self.failures = 0;
                Ok(())
            }
            Err(e) if is_permanent_pg_error(&e) => {
                self.failures += 1;
                if self.failures >= MAX_PERMANENT_RETRIES {
                    tracing::error!(
                        error = %e,
                        retries = self.failures,
                        "{}: permanent postgres error, skipping message",
                        self.label
                    );
                    self.failures = 0;
                    Ok(())
                } else {
                    Err(e)
                }
            }
            Err(e) => Err(e),
        }
    }

    async fn on_tick(&mut self) {
        self.inner.on_tick().await
    }

    async fn on_revoke(&mut self) {
        self.inner.on_revoke().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permanent_sqlstate_classification() {
        assert!(is_permanent_sqlstate("22021")); // invalid byte sequence (NUL)
        assert!(is_permanent_sqlstate("22001")); // string data right truncation
        assert!(is_permanent_sqlstate("22P02")); // invalid text representation
        assert!(is_permanent_sqlstate("54000")); // program limit exceeded (btree index row size)
        assert!(!is_permanent_sqlstate("40001")); // serialization failure (transient)
        assert!(!is_permanent_sqlstate("57P01")); // admin shutdown (transient)
        assert!(!is_permanent_sqlstate("08006")); // connection failure (transient)
    }
}
