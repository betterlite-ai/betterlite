mod tests;

use betterlite_config::sections::settings::{JsonValue, SettingsStore};

const SUPPORTED_DB_OBJECTS: &str = "supported_db_objects";

// LiteLLM types the kind as str | SupportedDBObjectType, and its loaders pass kinds the enum lacks
#[must_use]
pub fn should_load_db_object(general_settings: &SettingsStore, object_type: &str) -> bool {
    let Some(value) = general_settings.get(SUPPORTED_DB_OBJECTS) else {
        return true;
    };
    match value {
        JsonValue::Null => true,
        JsonValue::Array(entries) => entries
            .iter()
            .any(|entry| matches!(entry, JsonValue::String(name) if name == object_type)),
        JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) | JsonValue::Object(_) => {
            let kind = json_type_name(value);
            tracing::warn!("supported_db_objects is not a list, got {kind}. Loading all objects.");
            true
        }
    }
}

// LiteLLM writes Python's class repr here; betterlite names the JSON type instead
fn json_type_name(value: &JsonValue) -> &'static str {
    match value {
        JsonValue::Null => "null",
        JsonValue::Bool(_) => "a boolean",
        JsonValue::Number(_) => "a number",
        JsonValue::String(_) => "a string",
        JsonValue::Array(_) => "an array",
        JsonValue::Object(_) => "an object",
    }
}
