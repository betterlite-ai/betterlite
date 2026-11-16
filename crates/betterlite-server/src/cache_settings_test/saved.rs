use async_trait::async_trait;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_secrets::secret_codec::{DecryptPolicy, FailureLog, FailureValue, StoredValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Map, Value};

use crate::cache_settings::parse::parse_stored_settings;

mod tests;

pub(crate) const CACHE_CONFIG_ROW_ID: &str = "cache_config";

const SELECT_SETTINGS_SQL: &str =
    "SELECT cache_settings::text FROM \"LiteLLM_CacheConfig\" WHERE id = $1";

// LiteLLM's decrypt_value_helper(return_original_value=True) keeps a value it cannot open exactly as stored.
const DECRYPT_POLICY: DecryptPolicy = DecryptPolicy {
    log: FailureLog::Error,
    value: FailureValue::Original,
};

// B726 has no cache config repository of its own, so the read seam lives here; the Postgres adapter is its production side.
#[async_trait]
pub trait SavedCacheConfig: Send + Sync {
    async fn cache_settings(&self) -> Result<Option<Value>, TableError>;
}

#[async_trait]
impl SavedCacheConfig for PostgresStore {
    async fn cache_settings(&self) -> Result<Option<Value>, TableError> {
        let client = self.session().await?;
        let row = client
            .query_opt(SELECT_SETTINGS_SQL, &[&CACHE_CONFIG_ROW_ID])
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

// A saved row that cannot be read, parsed or decrypted leaves the saved map empty: none of those failures may block the connection test.
pub(crate) async fn saved_settings(
    repository: Option<&dyn SavedCacheConfig>,
    cipher: &DbCipher,
) -> Map<String, Value> {
    let Some(repository) = repository else {
        return Map::new();
    };
    let Ok(Some(stored)) = repository.cache_settings().await else {
        return Map::new();
    };
    let Ok(mut settings) = parse_stored_settings(&stored) else {
        return Map::new();
    };
    for (field, value) in &mut settings {
        decrypt_setting(value, field, cipher);
    }
    settings
}

fn decrypt_setting(value: &mut Value, field: &str, cipher: &DbCipher) {
    let Value::String(text) = value else {
        return;
    };
    let stored = StoredValue::<()>::Text(SecretString::from(text.clone()));
    if let Some(StoredValue::Text(opened)) = cipher.decrypt_value(stored, field, DECRYPT_POLICY) {
        opened.expose_secret().clone_into(text);
    }
}
