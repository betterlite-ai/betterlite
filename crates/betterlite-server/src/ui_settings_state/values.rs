use betterlite_config::sections::settings::settings_rules::JsonValue;
use betterlite_config::ui_settings::UiSettingsDict;
use betterlite_models::decisions::Json;

// The two JSON enums share one shape, so a single template writes the bridge both ways.
macro_rules! convert_json {
    (
        $name:ident, $source:ident, $target:ident, $source_text:ident, $target_text:ident,
        $source_array:ident, $target_array:ident
    ) => {
        #[must_use]
        pub fn $name(value: &$source) -> $target {
            match value {
                $source::Null => $target::Null,
                $source::Bool(flag) => $target::Bool(*flag),
                $source::Number(number) => $target::Number(number.clone()),
                $source::$source_text(text) => $target::$target_text(text.clone()),
                $source::$source_array(items) => {
                    $target::$target_array(items.iter().map($name).collect())
                }
                $source::Object(entries) => $target::Object(
                    entries
                        .iter()
                        .map(|(key, entry)| (key.clone(), $name(entry)))
                        .collect(),
                ),
            }
        }
    };
}

convert_json!(decision_json, JsonValue, Json, String, Text, Array, List);
convert_json!(settings_json, Json, JsonValue, Text, String, List, Array);

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
