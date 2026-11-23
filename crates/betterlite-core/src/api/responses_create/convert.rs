use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::collections::BTreeMap;
// The three JSON enums share one shape, so a re-encode carries a value between them without a converter per pair
pub fn reencode<T, U>(value: &T) -> Result<U, serde_json::Error>
where
    T: Serialize + ?Sized,
    U: DeserializeOwned,
{
    match serde_json::to_string(value) {
        Ok(text) => serde_json::from_str(&text),
        Err(error) => Err(error),
    }
}
// The three enums spell their list variant differently, so one template writes each direction
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
convert_json!(json_value, Json, JsonValue, List, Array);
convert_json!(decision_json, JsonValue, Json, Array, List);
convert_json!(prompt_value, JsonValue, PromptJson, Array, List);
convert_json!(value_from_prompt, PromptJson, JsonValue, List, Array);
convert_json!(decision_prompt, PromptJson, Json, List, List);
convert_json!(prompt_decision, Json, PromptJson, List, List);
#[must_use]
pub fn object_value(body: &Object) -> JsonValue {
    JsonValue::Object(
        body.iter()
            .map(|(name, value)| (name.clone(), value_from_prompt(value)))
            .collect(),
    )
}
#[must_use]
pub fn object_from_params(params: &BTreeMap<String, JsonValue>) -> Object {
    params
        .iter()
        .map(|(name, value)| (name.clone(), prompt_value(value)))
        .collect()
}
