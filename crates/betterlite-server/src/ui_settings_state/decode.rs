use std::collections::BTreeMap;

use betterlite_config::sections::settings::settings_rules::JsonValue;
use betterlite_config::ui_settings::UiSettingsDict;
use betterlite_db::budget_permission_rows::JsonData;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoredUiSettingsError {
    #[error("the stored UI settings are not valid JSON")]
    Json(#[from] serde_json::Error),
    #[error("the stored UI settings are not a JSON object")]
    NotAnObject,
}

// The ui_settings column is JSON; LiteLLM reads it as a mapping or as the JSON text of one.
pub fn decode_stored_ui_settings(bytes: &[u8]) -> Result<UiSettingsDict, StoredUiSettingsError> {
    let stored: JsonData = serde_json::from_slice(bytes)?;
    match stored {
        JsonData::Null => Ok(UiSettingsDict::new()),
        JsonData::Object(entries) => Ok(object_settings(entries)),
        JsonData::String(text) => match serde_json::from_str::<JsonData>(&text)? {
            JsonData::Object(entries) => Ok(object_settings(entries)),
            JsonData::Null
            | JsonData::Bool(_)
            | JsonData::Number(_)
            | JsonData::String(_)
            | JsonData::Array(_) => Err(StoredUiSettingsError::NotAnObject),
        },
        JsonData::Bool(_) | JsonData::Number(_) | JsonData::Array(_) => {
            Err(StoredUiSettingsError::NotAnObject)
        }
    }
}

fn object_settings(entries: BTreeMap<String, JsonData>) -> UiSettingsDict {
    entries
        .into_iter()
        .map(|(key, value)| (key, settings_value(value)))
        .collect()
}

fn settings_value(value: JsonData) -> JsonValue {
    match value {
        JsonData::Null => JsonValue::Null,
        JsonData::Bool(flag) => JsonValue::Bool(flag),
        JsonData::Number(number) => JsonValue::Number(number),
        JsonData::String(text) => JsonValue::String(text),
        JsonData::Array(items) => JsonValue::Array(items.into_iter().map(settings_value).collect()),
        JsonData::Object(entries) => JsonValue::Object(
            entries
                .into_iter()
                .map(|(key, entry)| (key, settings_value(entry)))
                .collect(),
        ),
    }
}
