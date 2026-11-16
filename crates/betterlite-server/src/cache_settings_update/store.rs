use async_trait::async_trait;
use betterlite_config::sections::settings::JsonValue;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::database_faults::native::is_native_transport_error;
use betterlite_db::database_faults::{DbError, ErrorType};
use betterlite_db::postgres_store::errors::backend;

use crate::cache_settings_read::stored::{
    CACHE_CONFIG_ROW_ID, CacheConfigStore, DatabaseCacheConfigStore,
};

const UPSERT_SQL: &str = "INSERT INTO \"LiteLLM_CacheConfig\" (id, cache_settings, updated_at) \
     VALUES ($1, $2::text::jsonb, now()) ON CONFLICT (id) \
     DO UPDATE SET cache_settings = EXCLUDED.cache_settings, updated_at = now()";

// The write half of B4307's cache config store: the row the POST route reads and saves.
#[async_trait]
pub trait CacheConfigRepository: Send + Sync {
    async fn stored_cache_settings(&self) -> Result<Option<JsonValue>, DbError>;

    async fn upsert_cache_settings(&self, settings_json: &str) -> Result<(), DbError>;
}

#[async_trait]
impl CacheConfigRepository for DatabaseCacheConfigStore {
    async fn stored_cache_settings(&self) -> Result<Option<JsonValue>, DbError> {
        <Self as CacheConfigStore>::stored_cache_settings(self)
            .await
            .map_err(database_fault)
    }

    async fn upsert_cache_settings(&self, settings_json: &str) -> Result<(), DbError> {
        let client = self
            .store
            .session()
            .await
            .map_err(|source| database_fault(TableError::Database(source)))?;
        client
            .execute(UPSERT_SQL, &[&CACHE_CONFIG_ROW_ID, &settings_json])
            .await
            .map_err(|source| database_fault(TableError::Database(backend(source))))?;
        Ok(())
    }
}

// B872 retries only the fault types it is given, so a transport failure has to arrive as Connect.
fn database_fault(error: TableError) -> DbError {
    let transport =
        matches!(&error, TableError::Database(native) if is_native_transport_error(native));
    DbError {
        error_type: if transport {
            ErrorType::Connect
        } else {
            ErrorType::Other
        },
        message: error.to_string(),
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    }
}
