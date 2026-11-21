use betterlite_cache::backend::memory::MemoryCacheOps;
use betterlite_cache::dual_cache::{CacheOptions, DualCache};
use betterlite_config::ui_settings::{ALLOWED_UI_SETTINGS_FIELDS, UiSettingsDict};
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_models::decisions::Json;
use thiserror::Error;
use tracing::warn;

use super::decode::{StoredUiSettingsError, decode_stored_ui_settings};
use super::projection::{GeneralSettingsTarget, apply_runtime_ui_flags};
use super::repository::UiSettingsRepository;
use super::values::{decision_object, settings_json};

pub const UI_SETTINGS_CACHE_KEY: &str = "ui_settings:settings_dict";
pub const UI_SETTINGS_CACHE_TTL_SECONDS: f64 = 600.0;

#[derive(Debug, Error)]
pub enum UiSettingsReadError {
    #[error(transparent)]
    Table(#[from] TableError),
    #[error(transparent)]
    Stored(#[from] StoredUiSettingsError),
}

// get_ui_settings_cached: the cached dict comes back as stored, while a database read is sanitized.
pub async fn get_ui_settings_cached<Memory: MemoryCacheOps>(
    cache: &DualCache<Memory>,
    repository: Option<&dyn UiSettingsRepository>,
) -> Result<UiSettingsDict, UiSettingsReadError> {
    let cached = cache.async_get_cache(UI_SETTINGS_CACHE_KEY, false).await;
    if let Some(settings) = cached.and_then(cached_settings) {
        return Ok(settings);
    }
    let Some(repository) = repository else {
        return Ok(UiSettingsDict::new());
    };
    let stored = match repository.stored_ui_settings().await? {
        Some(text) => decode_stored_ui_settings(text.as_bytes())?,
        None => UiSettingsDict::new(),
    };
    let sanitized: UiSettingsDict = stored
        .into_iter()
        .filter(|(key, _)| ALLOWED_UI_SETTINGS_FIELDS.contains(&key.as_str()))
        .collect();
    cache
        .async_set_cache(
            UI_SETTINGS_CACHE_KEY,
            &decision_object(&sanitized),
            CacheOptions {
                ttl: Some(UI_SETTINGS_CACHE_TTL_SECONDS),
                local_only: false,
                refresh_ttl: false,
            },
        )
        .await;
    Ok(sanitized)
}

// sync_ui_settings_to_general_settings: a read that fails leaves this pod on the flags it already had.
pub async fn sync_ui_settings(
    repository: &dyn UiSettingsRepository,
    target: GeneralSettingsTarget<'_>,
) -> UiSettingsDict {
    let decoded = match repository.stored_ui_settings().await {
        Ok(Some(text)) => {
            decode_stored_ui_settings(text.as_bytes()).map_err(UiSettingsReadError::from)
        }
        Ok(None) => Ok(UiSettingsDict::new()),
        Err(error) => Err(UiSettingsReadError::Table(error)),
    };
    match decoded {
        Ok(settings) => apply_runtime_ui_flags(&settings, target),
        Err(error) => {
            warn!(error = %error, "Could not refresh UI settings from the database");
            UiSettingsDict::new()
        }
    }
}

fn cached_settings(cached: Json) -> Option<UiSettingsDict> {
    match cached {
        Json::Object(entries) => Some(
            entries
                .iter()
                .map(|(key, value)| (key.clone(), settings_json(value)))
                .collect(),
        ),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}
