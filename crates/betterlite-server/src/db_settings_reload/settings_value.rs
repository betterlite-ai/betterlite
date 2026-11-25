use betterlite_config::sections::settings::JsonValue;
use betterlite_config::sections::settings::coerce_bool::coerce_bool;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::database_faults::native::is_native_transport_error;
use betterlite_db::database_faults::{DbError, ErrorType};
use betterlite_openai::openai_base_config::Object as JsonMap;

use super::SettingsReloadError;
use super::ports::SettingsRows;

// LiteLLM decodes a JSON string param_value once; every other shape fails at the .get it does next.
pub(crate) async fn litellm_settings_object<Db>(
    db: &Db,
) -> Result<Option<JsonMap>, SettingsReloadError>
where
    Db: SettingsRows + ?Sized,
{
    let Some(stored) = db.litellm_settings().await? else {
        return Ok(None);
    };
    match stored {
        JsonValue::Null => Ok(None),
        JsonValue::Object(settings) => Ok(Some(settings)),
        JsonValue::String(text) => decode_settings(&text),
        JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) => {
            Err(SettingsReloadError::LitellmSettingsNotAnObject)
        }
    }
}

fn decode_settings(text: &str) -> Result<Option<JsonMap>, SettingsReloadError> {
    let decoded: JsonValue = serde_json::from_str(text)?;
    match decoded {
        JsonValue::Object(settings) => Ok(Some(settings)),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::String(_)
        | JsonValue::Array(_) => Err(SettingsReloadError::LitellmSettingsNotAnObject),
    }
}

// coerce_bool then bool(), as the reload does: the stored flag may be any truthy value.
pub(crate) fn enabled_flag(config: &JsonMap) -> bool {
    let stored = config
        .get("enabled")
        .map_or(JsonValue::Bool(true), |value| coerce_bool(value.clone()));
    matches!(stored, JsonValue::Bool(true))
}

// A stored filter config must be a mapping; null is the same as no key at all.
pub(crate) fn semantic_config(stored: &JsonValue) -> Result<Option<&JsonMap>, SettingsReloadError> {
    if let JsonValue::Object(config) = stored {
        return Ok(Some(config));
    }
    if stored.is_null() {
        return Ok(None);
    }
    Err(SettingsReloadError::SemanticFilterNotAnObject)
}

// mcp_semantic_tool_filter is a declared bool, so a value of another type is an error naming the field.
pub(crate) fn semantic_enabled(config: &JsonMap) -> Result<bool, SettingsReloadError> {
    match config.get("enabled") {
        Some(JsonValue::Bool(flag)) => Ok(*flag),
        None | Some(JsonValue::Null) => Ok(false),
        Some(
            JsonValue::Number(_)
            | JsonValue::String(_)
            | JsonValue::Array(_)
            | JsonValue::Object(_),
        ) => Err(SettingsReloadError::SemanticFilterEnabledNotBool),
    }
}

// The retry helper classifies faults by type, so a native transport error has to arrive as Connect.
pub(crate) fn database_fault(error: &TableError) -> DbError {
    let transport =
        matches!(error, TableError::Database(native) if is_native_transport_error(native));
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
