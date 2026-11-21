//! `RunbookAuthor` – zamiana wolnego tekstu/Markdownu na kroki runbooka (zadanie 11.4, design D12).
//!
//! W MVP jest tylko dostawca `none`: runbook MANUAL zostaje tekstem. Dostawca LLM (poza MVP)
//! musiałby zwracać szkic do akceptacji człowieka, a kroki i tak przechodzą przez
//! `Registry::classify` (whitelista akcji, wymuszone zatwierdzenia).

use contracts::RunbookDef;

pub trait RunbookAuthor: Send + Sync {
    fn name(&self) -> &'static str;
    /// Szkic runbooka AUTOMATIC z tekstu; `None` = zostaw jako MANUAL.
    /// Nikt go jeszcze nie woła: import z LLM jest poza MVP (CLAUDE.md, „Odłożone”).
    #[allow(dead_code)]
    fn draft(&self, manual: &RunbookDef) -> Option<RunbookDef>;
}

/// Dostawca `none`: nic nie generuje.
pub struct NoAuthor;

impl RunbookAuthor for NoAuthor {
    fn name(&self) -> &'static str {
        "none"
    }

    fn draft(&self, _manual: &RunbookDef) -> Option<RunbookDef> {
        None
    }
}

/// Dostawca z `LLM_PROVIDER` (domyślnie `none`). Nieznana nazwa = błąd startu, nie cicha zmiana.
pub fn from_env() -> anyhow::Result<Box<dyn RunbookAuthor>> {
    match std::env::var("LLM_PROVIDER").as_deref().unwrap_or("none") {
        "none" | "" => Ok(Box::new(NoAuthor)),
        other => anyhow::bail!("LLM_PROVIDER={other} nie jest obsługiwany w MVP (dostępny: none)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_provider_keeps_manual_runbooks_as_text() {
        let author = from_env().unwrap();
        assert_eq!(author.name(), "none");
        let def = crate::parse::parse_runbook("p.md", "# Title\nDo things").unwrap();
        assert!(author.draft(&def).is_none());
    }
}
