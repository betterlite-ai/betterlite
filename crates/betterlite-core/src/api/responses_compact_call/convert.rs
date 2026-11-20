use betterlite_context::non_default_params::Params;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};

// The three JSON enums share one shape, so a single template writes each direction of the bridge
macro_rules! convert_json {
    ($name:ident, $source:ident, $target:ident, $list:ident) => {
        pub(super) fn $name(value: &$source) -> $target {
            match value {
                $source::Null => $target::Null,
                $source::Bool(flag) => $target::Bool(*flag),
                $source::Number(number) => $target::Number(number.clone()),
                $source::Text(text) => $target::Text(text.clone()),
                $source::$list(items) => $target::$list(items.iter().map($name).collect()),
                $source::Object(fields) => $target::Object(
                    fields
                        .iter()
                        .map(|(name, item)| (name.clone(), $name(item)))
                        .collect(),
                ),
            }
        }
    };
}

convert_json!(decision_json, JsonValue, Json, Array);
convert_json!(json_value, Json, JsonValue, Array);
convert_json!(prompt_decision, PromptJson, Json, List);
convert_json!(prompt_value, JsonValue, PromptJson, Array);

pub(super) fn object_from_params(params: &Params) -> Object {
    params
        .iter()
        .map(|(name, value)| (name.clone(), prompt_value(value)))
        .collect()
}
