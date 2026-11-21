use betterlite_config::sections::settings::JsonValue;
use betterlite_logging::private_json::error::PrivateJsonError;
use betterlite_logging::private_json::safe_json::safe_json_loads;
use thiserror::Error;

use super::store::coordination_block;

pub const SETTINGS_FIELD: &str = "settings";

#[derive(Debug, Error)]
pub enum DecodeError {
    #[error(transparent)]
    Bounded(#[from] PrivateJsonError),
    #[error("the coordination Redis document is not a JSON object")]
    NotAnObject,
    #[error("missing field `settings`")]
    MissingSettings,
    #[error("`settings` must be a JSON object")]
    SettingsNotAnObject,
}

// The request body of both coordination Redis routes: a required settings object and nothing else this layer reads.
pub fn decode_coordination_settings(bytes: &[u8]) -> Result<JsonValue, DecodeError> {
    let JsonValue::Object(members) = safe_json_loads(bytes)? else {
        return Err(DecodeError::NotAnObject);
    };
    match members.get(SETTINGS_FIELD) {
        Some(settings @ JsonValue::Object(_)) => Ok(settings.clone()),
        Some(_) => Err(DecodeError::SettingsNotAnObject),
        None => Err(DecodeError::MissingSettings),
    }
}

// A persisted general_settings row as JSON, read down to its coordination_redis block.
pub fn decode_persisted_settings(bytes: &[u8]) -> Result<Option<JsonValue>, DecodeError> {
    let document = safe_json_loads(bytes)?;
    if !document.is_object() {
        return Err(DecodeError::NotAnObject);
    }
    Ok(coordination_block(&document))
}
