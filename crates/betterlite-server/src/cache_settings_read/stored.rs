use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::auxiliary_repository::bindings::CacheConfig;
use betterlite_db::auxiliary_repository::repository::{
    PrismaClient, PrismaTableRepository, TableError,
};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_redis::config_sync::publish::redis_config_change_publisher;
use betterlite_secrets::secret_codec::{DecryptPolicy, FailureLog, FailureValue, StoredValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Map, Value};
use thiserror::Error;

use super::view::carries_value;
use crate::cache_settings::parse::{ParseStoredSettingsError, parse_stored_settings};

pub const CACHE_CONFIG_ROW_ID: &str = "cache_config";

const SELECT_SQL: &str = r#"SELECT cache_settings::text FROM "LiteLLM_CacheConfig" WHERE id = $1"#;

// decrypt_value_helper(return_original_value=True): a value that does not decrypt stays as stored.
const KEEP_ORIGINAL: DecryptPolicy = DecryptPolicy {
    log: FailureLog::Debug,
    value: FailureValue::Original,
};

#[derive(Debug, Error)]
pub enum StoredSettingsError {
    #[error(transparent)]
    Table(#[from] TableError),
    #[error(transparent)]
    Parse(#[from] ParseStoredSettingsError),
}

// The cache_config row, through the repository so reads follow the configured database replica.
#[async_trait]
pub trait CacheConfigStore: Send + Sync {
    async fn stored_cache_settings(&self) -> Result<Option<Value>, TableError>;
}

pub struct DatabaseCacheConfigStore {
    pub store: Arc<PostgresStore>,
    pub availability: Availability,
}

#[async_trait]
impl CacheConfigStore for DatabaseCacheConfigStore {
    async fn stored_cache_settings(&self) -> Result<Option<Value>, TableError> {
        let client = PrismaClient {
            writer: self.store.as_ref(),
            reader: self.store.as_ref(),
            availability: &self.availability,
        };
        let repository = PrismaTableRepository::new(
            Some(client),
            redis_config_change_publisher(None),
            CacheConfig,
        );
        let access = repository.access()?;
        let session = access.read.session().await?;
        let row = session
            .query_opt(SELECT_SQL, &[&CACHE_CONFIG_ROW_ID])
            .await
            .map_err(|source| TableError::Database(backend(source)))?;
        let Some(found) = row else {
            return Ok(None);
        };
        let text: String = found
            .try_get(0)
            .map_err(|source| TableError::Database(backend(source)))?;
        serde_json::from_str(&text)
            .map(Some)
            .map_err(TableError::Row)
    }
}

// The truthiness check, the parse and the decryption of one raw cache_settings column.
pub fn stored_from_raw(
    raw: &Value,
    cipher: &DbCipher,
) -> Result<Map<String, Value>, StoredSettingsError> {
    if !carries_value(raw) {
        return Ok(Map::new());
    }
    let parsed = parse_stored_settings(raw)?;
    Ok(parsed
        .into_iter()
        .map(|(field, value)| {
            let decrypted = decrypt_value(cipher, &field, value);
            (field, decrypted)
        })
        .collect())
}

pub async fn stored_settings(
    store: &dyn CacheConfigStore,
    cipher: &DbCipher,
) -> Result<Map<String, Value>, StoredSettingsError> {
    let Some(raw) = store.stored_cache_settings().await? else {
        return Ok(Map::new());
    };
    stored_from_raw(&raw, cipher)
}

fn decrypt_value(cipher: &DbCipher, field: &str, value: Value) -> Value {
    match value {
        Value::String(text) => {
            let stored = StoredValue::Text(SecretString::from(text));
            match cipher.decrypt_value(stored, field, KEEP_ORIGINAL) {
                Some(StoredValue::Text(plain)) => Value::String(plain.expose_secret().to_owned()),
                Some(StoredValue::Other(other)) => other,
                None => Value::Null,
            }
        }
        other => other,
    }
}

mod tests;
