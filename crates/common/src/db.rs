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

/// Łączy się z bazą (z ponawianiem, bo w compose Postgres może jeszcze startować)
/// i wykonuje wszystkie migracje.
pub async fn connect_and_migrate(database_url: &str) -> anyhow::Result<PgPool> {
    let pool = connect(database_url).await?;
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .context("running migrations")?;
    tracing::info!("database migrations applied");
    Ok(pool)
}

/// Łączy się z bazą bez migracji.
pub async fn connect(database_url: &str) -> anyhow::Result<PgPool> {
    let mut attempt = 0;
    loop {
        match PgPoolOptions::new()
            .max_connections(10)
            .acquire_timeout(Duration::from_secs(5))
            .connect(database_url)
            .await
        {
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
