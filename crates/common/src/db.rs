//! Postgres: pula połączeń i migracje.
//!
//! Migracje leżą w `migrations/` w korzeniu repo i są wkompilowane w każdą binarkę.
//! Konwencja nazw (sqlx wymaga numeru wersji na początku):
//!   P1: `1001_p1_<opis>.sql` … `1999_…`
//!   P2: `2001_p2_<opis>.sql` … `2999_…`
//!   P3: `3001_p3_<opis>.sql` … `3999_…`
//! Każdy strumień numeruje tylko w swoim zakresie, więc nie ma konfliktów nazw.
//! Migracje uruchamia każda binarka przy starcie; równoległe starty są bezpieczne
//! (sqlx bierze advisory lock), a brakujące wersje z niższego zakresu są dokładane.

use std::time::Duration;

use anyhow::Context;
use sqlx::postgres::{PgPool, PgPoolOptions};

pub use sqlx;

/// Domyślny rozmiar puli, gdy serwis nie podał własnego (`connect`/`connect_and_migrate`).
pub const DEFAULT_MAX_CONNECTIONS: u32 = 10;

/// Rozmiar puli: `DB_MAX_CONNECTIONS` (≥1) i `DB_MIN_CONNECTIONS` (≥0) z env mają
/// pierwszeństwo przed domyślną wartością serwisu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolSize {
    pub max: u32,
    pub min: u32,
}

impl PoolSize {
    /// Rozmiar z env; `default_max` obowiązuje, gdy zmiennej brak lub jest niepoprawna.
    pub fn from_env(default_max: u32) -> Self {
        Self::parse(
            std::env::var("DB_MAX_CONNECTIONS").ok().as_deref(),
            std::env::var("DB_MIN_CONNECTIONS").ok().as_deref(),
            default_max,
        )
    }

    /// Czysta logika parsowania (testowalna bez env i bazy). Niepoprawna wartość
    /// (nie liczba, `DB_MAX_CONNECTIONS=0`) -> ostrzeżenie i wartość domyślna.
    pub fn parse(max_raw: Option<&str>, min_raw: Option<&str>, default_max: u32) -> Self {
        let default_max = default_max.max(1);
        let max = match max_raw.map(str::trim) {
            None | Some("") => default_max,
            Some(v) => match v.parse::<u32>() {
                Ok(n) if n >= 1 => n,
                _ => {
                    tracing::warn!(
                        value = v,
                        default_max,
                        "invalid DB_MAX_CONNECTIONS, using default"
                    );
                    default_max
                }
            },
        };
        let min = match min_raw.map(str::trim) {
            None | Some("") => 0,
            Some(v) => v.parse::<u32>().unwrap_or_else(|_| {
                tracing::warn!(value = v, "invalid DB_MIN_CONNECTIONS, using 0");
                0
            }),
        };
        Self {
            max,
            min: min.min(max),
        }
    }
}

/// Łączy się z bazą (domyślna pula 10) i wykonuje migracje.
pub async fn connect_and_migrate(database_url: &str) -> anyhow::Result<PgPool> {
    connect_and_migrate_with(database_url, DEFAULT_MAX_CONNECTIONS).await
}

/// Jak `connect_and_migrate`, z domyślnym maksimum puli właściwym dla serwisu
/// (nadpisywanym przez `DB_MAX_CONNECTIONS`).
pub async fn connect_and_migrate_with(
    database_url: &str,
    default_max: u32,
) -> anyhow::Result<PgPool> {
    let pool = connect_with(database_url, default_max).await?;
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .context("running migrations")?;
    tracing::info!("database migrations applied");
    Ok(pool)
}

/// Łączy się z bazą bez migracji (domyślna pula 10).
pub async fn connect(database_url: &str) -> anyhow::Result<PgPool> {
    connect_with(database_url, DEFAULT_MAX_CONNECTIONS).await
}

/// Łączy się z bazą (z ponawianiem, bo w compose Postgres może jeszcze startować).
pub async fn connect_with(database_url: &str, default_max: u32) -> anyhow::Result<PgPool> {
    let size = PoolSize::from_env(default_max);
    tracing::info!(
        max_connections = size.max,
        min_connections = size.min,
        "postgres pool"
    );
    let mut attempt = 0;
    loop {
        let opts = PgPoolOptions::new()
            .max_connections(size.max)
            .min_connections(size.min)
            .acquire_timeout(Duration::from_secs(5));
        match opts.connect(database_url).await {
            Ok(pool) => return Ok(pool),
            Err(e) if attempt < 30 => {
                attempt += 1;
                tracing::warn!(error = %e, attempt, "postgres not ready, retrying");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Err(e) => return Err(e).context("connecting to postgres"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PoolSize;

    #[test]
    fn default_when_unset_or_empty() {
        assert_eq!(PoolSize::parse(None, None, 8), PoolSize { max: 8, min: 0 });
        assert_eq!(
            PoolSize::parse(Some(" "), Some(""), 8),
            PoolSize { max: 8, min: 0 }
        );
    }

    #[test]
    fn env_overrides_default() {
        assert_eq!(
            PoolSize::parse(Some("3"), Some("1"), 8),
            PoolSize { max: 3, min: 1 }
        );
        assert_eq!(PoolSize::parse(Some(" 12 "), None, 8).max, 12);
    }

    #[test]
    fn invalid_falls_back_to_default() {
        assert_eq!(PoolSize::parse(Some("0"), None, 8).max, 8);
        assert_eq!(PoolSize::parse(Some("abc"), None, 8).max, 8);
        assert_eq!(
            PoolSize::parse(Some("-2"), Some("x"), 8),
            PoolSize { max: 8, min: 0 }
        );
    }

    #[test]
    fn min_is_clamped_to_max() {
        assert_eq!(
            PoolSize::parse(Some("2"), Some("5"), 8),
            PoolSize { max: 2, min: 2 }
        );
    }
}
