use serde_json::{Map, Value};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseStoredSettingsError {
    #[error("stored cache_settings is not valid JSON")]
    InvalidJson {
        #[source]
        source: serde_json::Error,
    },
}

// Stored cache_settings arrives as a JSON string or an already-parsed mapping; only the string form is decoded again.
pub fn parse_stored_settings(
    value: &Value,
) -> Result<Map<String, Value>, ParseStoredSettingsError> {
    let parsed = match value {
        Value::String(text) => serde_json::from_str(text)
            .map_err(|source| ParseStoredSettingsError::InvalidJson { source })?,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_) => {
            value.clone()
        }
    };
    Ok(match parsed {
        Value::Object(map) => map,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => {
            Map::new()
        }
    })
}
