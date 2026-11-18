use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::resource_repository::config::ConfigError;
use betterlite_db::resource_repository::config::ConfigRepository;

#[async_trait]
pub trait SettingsRepository: Send + Sync {
    async fn param(&self, name: &str) -> Result<Option<JsonData>, ConfigError>;
    async fn set_param(&self, name: &str, value: JsonData) -> Result<(), ConfigError>;
}

pub struct DatabaseSettingsRepository {
    pub store: Arc<PostgresStore>,
    pub availability: Availability,
}

#[async_trait]
impl SettingsRepository for DatabaseSettingsRepository {
    async fn param(&self, name: &str) -> Result<Option<JsonData>, ConfigError> {
        let client = PrismaClient {
            writer: self.store.as_ref(),
            reader: self.store.as_ref(),
            availability: &self.availability,
        };
        // LiteLLM builds the repository with use_writer=True, so the row is read from the writer too
        let repository = ConfigRepository::new(Some(client), true);
        let parameter = repository.get_param(name).await?;
        Ok(parameter.map(|param| param.param_value))
    }

    async fn set_param(&self, name: &str, value: JsonData) -> Result<(), ConfigError> {
        let client = PrismaClient {
            writer: self.store.as_ref(),
            reader: self.store.as_ref(),
            availability: &self.availability,
        };
        let repository = ConfigRepository::new(Some(client), true);
        repository.set_param(name, value).await.map(|_param| ())
    }
}
