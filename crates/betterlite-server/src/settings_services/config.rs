use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;
use thiserror::Error;

pub(crate) const LITELLM_SETTINGS: &str = "litellm_settings";

// A config value read as a settings mapping, named so the error says which field holds the wrong type
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SettingsConfigError {
    #[error("{field} must be a JSON object")]
    NotAnObject { field: String },
}

// _get_settings_with_schema's config read: an absent or null litellm_settings or settings block reads as empty before the record's defaults
pub fn settings_block(
    config: &BTreeMap<String, JsonValue>,
    setting_name: &str,
) -> Result<BTreeMap<String, JsonValue>, SettingsConfigError> {
    object_entries(
        object_entries(config.get(LITELLM_SETTINGS), LITELLM_SETTINGS)?.get(setting_name),
        setting_name,
    )
}

// config.get(field) or {}: an absent or null field is an empty mapping and any other non-object is refused
pub(crate) fn object_entries(
    value: Option<&JsonValue>,
    field: &str,
) -> Result<BTreeMap<String, JsonValue>, SettingsConfigError> {
    match value {
        None | Some(JsonValue::Null) => Ok(BTreeMap::new()),
        Some(JsonValue::Object(entries)) => Ok(entries
            .iter()
            .map(|(key, entry)| (key.clone(), entry.clone()))
            .collect()),
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) | JsonValue::Array(_),
        ) => Err(SettingsConfigError::NotAnObject {
            field: field.to_owned(),
        }),
    }
}
