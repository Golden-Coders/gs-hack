//! Shared application state for the API server.

use std::sync::Arc;

use common::db::sqlx::PgPool;
use common::kafka::rdkafka::producer::FutureProducer;
use runbooks::author::RunbookAuthor;

use crate::auth::AuthStore;

/// Domyślny maks. rozmiar puli Postgresa (nadpisywany przez `DB_MAX_CONNECTIONS`, zob. docs/LOADTEST.md).
const DB_POOL_DEFAULT: u32 = 5;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<common::Config>,
    pub db: PgPool,
    pub kafka: FutureProducer,
    pub author: Arc<dyn RunbookAuthor>,
    pub auth: AuthStore,
}

impl AppState {
    pub async fn connect(config: common::Config) -> anyhow::Result<Self> {
        let db =
            common::db::connect_and_migrate_with(&config.database_url, DB_POOL_DEFAULT).await?;
        crate::auth::ensure_auth_users_seeded(&db, &config.config_dir).await;
        let kafka = common::kafka::producer(&config.kafka_brokers)?;
        let author = runbooks::author::from_env()?;
        let auth = AuthStore::from_env();
        tracing::info!(
            provider = author.name(),
            model = author.model(),
            "api runbook author"
        );
        Ok(Self {
            config: Arc::new(config),
            db,
            kafka,
            author,
            auth,
        })
    }
}
