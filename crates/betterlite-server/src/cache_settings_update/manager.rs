use std::sync::{Mutex, PoisonError};

use betterlite_config::sections::settings::JsonValue;
use betterlite_db::database_faults::DbError;
use betterlite_db::database_faults::reconnect::{
    DbReconnect, RetryPolicy, call_with_db_reconnect_retry,
};
use betterlite_db::stored_cipher::DbCipher;
use betterlite_models::decisions::{Extra, Json};
use serde_json::Number;
use thiserror::Error;

use super::store::CacheConfigRepository;
use crate::cache_settings_read::stored::{StoredSettingsError, stored_from_raw};
use crate::cache_settings_read::view::carries_value;

pub const REDIS_TYPE_FIELD: &str = "redis_type";
const LOOKUP_FAILURE_REASON: &str = "init_cache_settings_in_db_lookup_failure";
const INIT_LOG_PREFIX: &str = "litellm.proxy.management_endpoints.cache_settings_endpoints.py::CacheSettingsManager::init_cache_settings_in_db - ";

pub type CacheParams = Extra;

#[derive(Debug, Error)]
pub enum CacheReinitError {
    #[error("the cache could not be initialised: {message}")]
    Init { message: String },
}

pub trait CacheReinit: Send + Sync {
    fn init_cache(&self, params: &CacheParams) -> Result<(), CacheReinitError>;

    fn switch_on_response_caching(&self);
}

// LiteLLM's class variable `_last_cache_params`: one value the route and the database reload share.
#[derive(Debug, Default)]
pub struct CacheSettingsManager {
    last_params: Mutex<Option<CacheParams>>,
}

impl CacheSettingsManager {
    #[must_use]
    pub fn last_params(&self) -> Option<CacheParams> {
        self.last_params
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn update_params(&self, params: CacheParams) {
        let mut last = self
            .last_params
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *last = Some(params);
    }

    pub async fn init_from_db<Client>(
        &self,
        repository: &dyn CacheConfigRepository,
        cipher: &DbCipher,
        reinit: &dyn CacheReinit,
        reconnect: Option<&mut Client>,
    ) where
        Client: DbReconnect + Send,
    {
        if let Err(error) = self.reload(repository, cipher, reinit, reconnect).await {
            tracing::error!("{INIT_LOG_PREFIX}{error}");
        }
    }

    async fn reload<Client>(
        &self,
        repository: &dyn CacheConfigRepository,
        cipher: &DbCipher,
        reinit: &dyn CacheReinit,
        reconnect: Option<&mut Client>,
    ) -> Result<(), InitError>
    where
        Client: DbReconnect + Send,
    {
        let stored = call_with_db_reconnect_retry(
            reconnect,
            move || async move { repository.stored_cache_settings().await },
            RetryPolicy {
                reason: LOOKUP_FAILURE_REASON,
                retry_safe_error_types: None,
                timeout_seconds: None,
                lock_timeout_seconds: None,
            },
        )
        .await?;
        let Some(raw) = stored else {
            return Ok(());
        };
        if !carries_value(&raw) {
            return Ok(());
        }
        let decrypted = stored_from_raw(&stored_object(&raw)?, cipher)?;
        let mut params: CacheParams = serde_json::from_value(JsonValue::Object(decrypted))?;
        params.shift_remove(REDIS_TYPE_FIELD);
        if let Some(last) = self.last_params()
            && params_equal(&last, &params)
        {
            tracing::debug!("Cache settings unchanged, skipping reinitialization");
            return Ok(());
        }
        reinit.init_cache(&params)?;
        self.update_params(params);
        reinit.switch_on_response_caching();
        tracing::info!("Cache settings initialized from database");
        Ok(())
    }
}

// LiteLLM's stored-settings adapter: a JSON string must decode to an object, an already-parsed object is used as is.
fn stored_object(raw: &JsonValue) -> Result<JsonValue, InitError> {
    match raw {
        JsonValue::Object(_) => Ok(raw.clone()),
        JsonValue::String(text) => {
            let parsed: JsonValue = serde_json::from_str(text)?;
            if matches!(parsed, JsonValue::Object(_)) {
                Ok(parsed)
            } else {
                Err(InitError::NotAnObject)
            }
        }
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) => {
            Err(InitError::NotAnObject)
        }
    }
}

#[derive(Debug, Error)]
enum InitError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    Stored(#[from] StoredSettingsError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Reinit(#[from] CacheReinitError),
    #[error("stored cache settings are not a JSON object")]
    NotAnObject,
}

#[must_use]
pub fn params_equal(left: &CacheParams, right: &CacheParams) -> bool {
    let normalized_left = normalized(left);
    let normalized_right = normalized(right);
    normalized_left == normalized_right
}

fn normalized(params: &CacheParams) -> CacheParams {
    params
        .iter()
        .filter(|(key, value)| key.as_str() != REDIS_TYPE_FIELD && !matches!(value, Json::Null))
        .map(|(key, value)| (key.clone(), compared_value(value)))
        .collect()
}

fn compared_value(value: &Json) -> Json {
    match value {
        Json::Bool(flag) => Json::Text(if *flag { "True" } else { "False" }.to_owned()),
        Json::Number(number) => Json::Text(python_number_text(number)),
        Json::Null | Json::Text(_) | Json::List(_) | Json::Object(_) => value.clone(),
    }
}

fn python_number_text(number: &Number) -> String {
    if let Some(value) = number.as_i64() {
        return value.to_string();
    }
    if let Some(value) = number.as_u64() {
        return value.to_string();
    }
    if let Some(value) = number.as_i128() {
        return value.to_string();
    }
    if let Some(value) = number.as_u128() {
        return value.to_string();
    }
    number
        .as_f64()
        .map_or_else(|| number.to_string(), |value| value.to_string())
}

mod tests;
