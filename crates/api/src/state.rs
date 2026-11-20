//! Wspólny stan aplikacji (Faza 0).

use std::sync::Arc;

use common::db::sqlx::PgPool;
use common::kafka::rdkafka::producer::FutureProducer;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<common::Config>,
    /// Odczyty z tabel wszystkich strumieni; zapisy konfiguracji – tylko do tabel właściciela.
    pub db: PgPool,
    /// Publikacja komend (`alerts.commands`, `runbooks.commands`).
    pub kafka: FutureProducer,
}

impl AppState {
    pub async fn connect(config: common::Config) -> anyhow::Result<Self> {
        let db = common::db::connect_and_migrate(&config.database_url).await?;
        let kafka = common::kafka::producer(&config.kafka_brokers)?;
        Ok(Self { config: Arc::new(config), db, kafka })
    }
}
