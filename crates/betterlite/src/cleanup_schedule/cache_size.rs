use std::collections::BTreeMap;

use betterlite_config::sections::Section as _;
use betterlite_config::sections::auth_cache;
use betterlite_config::sections::settings::JsonValue;

use super::error::CleanupScheduleError;
use super::ports::{CacheMaxSize, GeneralSettingsView};

pub(super) const CACHE_MAX_SIZE_KEY: &str = "user_api_key_cache_max_size";

pub fn on_cache_size_changed(
    db_values: &BTreeMap<String, JsonValue>,
    cache_size_was_db: bool,
    store: &mut dyn GeneralSettingsView,
    cache: &dyn CacheMaxSize,
) -> Result<(), CleanupScheduleError> {
    if !db_values.contains_key(CACHE_MAX_SIZE_KEY) && !cache_size_was_db {
        return Ok(());
    }
    let writable = !store.owned_by_config(CACHE_MAX_SIZE_KEY);
    let value = store.get(CACHE_MAX_SIZE_KEY);
    match cache_max_size(value) {
        CacheSizeValue::Invalid => {
            if writable {
                store.remove_key(CACHE_MAX_SIZE_KEY)?;
            }
            let shown = value.map_or_else(|| "None".to_owned(), ToString::to_string);
            tracing::warn!(
                "Ignoring invalid general_settings.user_api_key_cache_max_size={shown} from the DB"
            );
        }
        CacheSizeValue::Unset => {
            if writable {
                store.remove_key(CACHE_MAX_SIZE_KEY)?;
            }
            cache.resize(None);
        }
        CacheSizeValue::Size(size) => {
            if writable {
                let written = u64::try_from(size).unwrap_or(u64::MAX);
                store.set(CACHE_MAX_SIZE_KEY, JsonValue::from(written))?;
            }
            cache.resize(Some(size));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CacheSizeValue {
    Unset,
    Size(usize),
    Invalid,
}

// The maximum size goes through the same auth cache section the config file uses, so both agree on what is valid.
fn cache_max_size(value: Option<&JsonValue>) -> CacheSizeValue {
    let document = serde_json::json!({
        "general_settings": { "user_api_key_cache_max_size": value },
    });
    let Ok(raw) = serde_json::from_value::<auth_cache::Raw>(document) else {
        return CacheSizeValue::Invalid;
    };
    let lookup = BTreeMap::<String, String>::new();
    let Ok(settings) = raw.convert(&lookup) else {
        return CacheSizeValue::Invalid;
    };
    settings
        .user_api_key_cache_max_size
        .map_or(CacheSizeValue::Unset, CacheSizeValue::Size)
}
