use crate::loader::ConfigError;
use crate::sections::settings::JsonValue;

const COLD_STORAGE_PATH: &str = "litellm_settings.cold_storage_custom_logger";

// litellm.cold_storage_custom_logger is an optional callback name, so only a string or a null loads
pub(super) fn custom_logger(configured: &JsonValue) -> Result<Option<String>, ConfigError> {
    match configured {
        JsonValue::Null => Ok(None),
        JsonValue::String(name) => Ok(Some(name.clone())),
        JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) | JsonValue::Object(_) => {
            Err(ConfigError::InvalidScalar {
                path: COLD_STORAGE_PATH.to_owned(),
                value: false,
            })
        }
    }
}
