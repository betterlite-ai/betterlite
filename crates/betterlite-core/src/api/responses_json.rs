use betterlite_context::non_default_params::Params;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};

// The three JSON enums share one shape, so a single template writes each direction of the bridge
macro_rules! convert_json {
    ($name:ident, $source:ident, $target:ident, $source_list:ident, $target_list:ident) => {
        #[must_use]
        pub fn $name(value: &$source) -> $target {
            match value {
                $source::Null => $target::Null,
                $source::Bool(flag) => $target::Bool(*flag),
                $source::Number(number) => $target::Number(number.clone()),
                $source::Text(text) => $target::Text(text.clone()),
                $source::$source_list(items) => {
                    $target::$target_list(items.iter().map($name).collect())
                }
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

convert_json!(decision_json, JsonValue, Json, Array, List);
convert_json!(json_value, Json, JsonValue, List, Array);
convert_json!(prompt_decision, PromptJson, Json, List, List);
convert_json!(prompt_value, JsonValue, PromptJson, Array, List);
convert_json!(decision_prompt, Json, PromptJson, List, List);
convert_json!(value_from_prompt, PromptJson, JsonValue, List, Array);

#[must_use]
pub fn object_value(body: &Object) -> JsonValue {
    JsonValue::Object(
        body.iter()
            .map(|(name, value)| (name.clone(), value_from_prompt(value)))
            .collect(),
    )
}

#[must_use]
pub fn object_from_params(params: &Params) -> Object {
    params
        .iter()
        .map(|(name, value)| (name.clone(), prompt_value(value)))
        .collect()
}
