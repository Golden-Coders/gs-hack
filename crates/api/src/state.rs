//! Shared application state for the API server.

use std::sync::Arc;

use common::db::sqlx::PgPool;
use common::kafka::rdkafka::producer::FutureProducer;

/// Domyślny maks. rozmiar puli Postgresa (nadpisywany przez `DB_MAX_CONNECTIONS`, zob. docs/LOADTEST.md).
const DB_POOL_DEFAULT: u32 = 5;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<common::Config>,
    pub db: PgPool,
    pub kafka: FutureProducer,
}

impl AppState {
    pub async fn connect(config: common::Config) -> anyhow::Result<Self> {
        let db = common::db::connect_and_migrate_with(&config.database_url, DB_POOL_DEFAULT).await?;
        let kafka = common::kafka::producer(&config.kafka_brokers)?;
        Ok(Self { config: Arc::new(config), db, kafka })
    }
}
