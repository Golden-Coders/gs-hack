//! Wspólne narzędzia przeładowywania konfiguracji z plików i powiadamiania o zmianach.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// Interwał pollingu plików konfiguracji (bind-mount Dockera na macOS nie przekazuje zdarzeń systemu plików).
pub const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Zwraca posortowaną listę plików `.yaml`/`.yml` w podanym katalogu (pomija dowiązania symboliczne).
pub fn yaml_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let mut files: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            matches!(p.extension().and_then(|e| e.to_str()), Some("yaml" | "yml"))
                && std::fs::symlink_metadata(p)
                    .map(|m| !m.file_type().is_symlink() && m.is_file())
                    .unwrap_or(false)
        })
        .collect();
    files.sort();
    files
}

/// Wysyła `SELECT pg_notify('config_changed', $1)` w Postgresie; błąd loguje jako warn.
#[cfg(feature = "db")]
pub async fn notify_config_changed(pool: &sqlx::PgPool, kind: &str) {
    if let Err(e) = sqlx::query("SELECT pg_notify('config_changed', $1)")
        .bind(kind)
        .execute(pool)
        .await
    {
        tracing::warn!(error = %e, kind, "pg_notify config_changed failed");
    }
}

/// Publikuje zdarzenie `UiEventKind::ConfigChanged` na topik `ui.events`; błąd loguje jako warn.
#[cfg(feature = "kafka")]
pub async fn publish_config_changed(producer: &rdkafka::producer::FutureProducer, kind: &str) {
    let ev = contracts::UiEvent {
        alert_id: None,
        kind: contracts::UiEventKind::ConfigChanged,
        payload: serde_json::json!({ "kind": kind }),
        at: chrono::Utc::now(),
    };
    if let Err(e) = crate::kafka::publish(
        producer,
        contracts::topics::UI_EVENTS,
        "config_changed",
        &ev,
    )
    .await
    {
        tracing::warn!(error = %e, kind, "publishing config_changed failed");
    }
}
