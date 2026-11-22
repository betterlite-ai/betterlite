use async_trait::async_trait;
use betterlite_db::auxiliary_repository::bindings::UISettings;
use betterlite_db::auxiliary_repository::repository::{
    PrismaClient, PrismaTableRepository, TableError,
};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_redis::config_sync::publish::redis_config_change_publisher;

pub const UI_SETTINGS_ROW_ID: &str = "ui_settings";

const SELECT_SQL: &str = r#"SELECT ui_settings::text FROM "LiteLLM_UISettings" WHERE id = $1"#;

// The ui_settings row of LiteLLM_UISettings, read the way the user banner read reads its own row.
#[async_trait]
pub trait UiSettingsRepository: Send + Sync {
    async fn stored_ui_settings(&self) -> Result<Option<String>, TableError>;
}

pub struct DatabaseUiSettingsRepository<'db> {
    pub store: &'db PostgresStore,
    pub availability: &'db Availability,
}

#[async_trait]
impl UiSettingsRepository for DatabaseUiSettingsRepository<'_> {
    async fn stored_ui_settings(&self) -> Result<Option<String>, TableError> {
        let client = PrismaClient {
            writer: self.store,
            reader: self.store,
            availability: self.availability,
        };
        let repository = PrismaTableRepository::new(
            Some(client),
            redis_config_change_publisher(None),
            UISettings,
        );
        let access = repository.access()?;
        let session = access.read.session().await?;
        let row = session
            .query_opt(SELECT_SQL, &[&UI_SETTINGS_ROW_ID])
            .await
            .map_err(|source| TableError::Database(backend(source)))?;
        let Some(found) = row else {
            return Ok(None);
        };
        // LiteLLM's db_record.ui_settings is nullable, and a NULL column reads as the empty settings.
        found
            .try_get(0)
            .map_err(|source| TableError::Database(backend(source)))
    }
}
