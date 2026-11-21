use betterlite_config::sections::settings::settings_rules::JsonValue;
use betterlite_config::ui_settings::UiSettingsDict;
use betterlite_models::decisions::Json;

// The dual cache stores LiteLLM's decisions JSON; the UI settings dict crosses that boundary both ways.
#[must_use]
pub fn decision_object(settings: &UiSettingsDict) -> Json {
    Json::Object(
        settings
            .iter()
            .map(|(key, value)| (key.clone(), decision_json(value)))
            .collect(),
    )
}

#[must_use]
pub fn decision_json(value: &JsonValue) -> Json {
    match value {
        JsonValue::Null => Json::Null,
        JsonValue::Bool(flag) => Json::Bool(*flag),
        JsonValue::Number(number) => Json::Number(number.clone()),
        JsonValue::String(text) => Json::Text(text.clone()),
        JsonValue::Array(items) => Json::List(items.iter().map(decision_json).collect()),
        JsonValue::Object(entries) => Json::Object(
            entries
                .iter()
                .map(|(key, value)| (key.clone(), decision_json(value)))
                .collect(),
        ),
    }
}

#[must_use]
pub fn settings_json(value: &Json) -> JsonValue {
    match value {
        Json::Null => JsonValue::Null,
        Json::Bool(flag) => JsonValue::Bool(*flag),
        Json::Number(number) => JsonValue::Number(number.clone()),
        Json::Text(text) => JsonValue::String(text.clone()),
        Json::List(items) => JsonValue::Array(items.iter().map(settings_json).collect()),
        Json::Object(entries) => JsonValue::Object(
            entries
                .iter()
                .map(|(key, value)| (key.clone(), settings_json(value)))
                .collect(),
        ),
    }
}
