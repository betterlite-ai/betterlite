use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::auxiliary_repository::bindings::SSOConfig;
use betterlite_db::auxiliary_repository::repository::{
    PrismaClient, PrismaTableRepository, TableError,
};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_redis::config_sync::publish::redis_config_change_publisher;

pub const SSO_CONFIG_ROW_ID: &str = "sso_config";

const SELECT_SQL: &str = r#"SELECT sso_settings::text FROM "LiteLLM_SSOConfig" WHERE id = $1"#;

// the sso_settings JSON of the settings row, read through the repository so replica routing applies
#[async_trait]
pub trait SsoSettingsRows: Send + Sync {
    async fn stored_sso_settings(&self) -> Result<Option<String>, TableError>;
}

pub struct DatabaseSsoSettings {
    pub store: Arc<PostgresStore>,
    pub availability: Availability,
}

#[async_trait]
impl SsoSettingsRows for DatabaseSsoSettings {
    async fn stored_sso_settings(&self) -> Result<Option<String>, TableError> {
        let client = PrismaClient {
            writer: self.store.as_ref(),
            reader: self.store.as_ref(),
            availability: &self.availability,
        };
        let repository = PrismaTableRepository::new(
            Some(client),
            redis_config_change_publisher(None),
            SSOConfig,
        );
        let access = repository.access()?;
        let session = access.read.session().await?;
        let row = session
            .query_opt(SELECT_SQL, &[&SSO_CONFIG_ROW_ID])
            .await
            .map_err(|source| TableError::Database(backend(source)))?;
        let Some(found) = row else {
            return Ok(None);
        };
        let text: String = found
            .try_get(0)
            .map_err(|source| TableError::Database(backend(source)))?;
        Ok(Some(text))
    }
}
