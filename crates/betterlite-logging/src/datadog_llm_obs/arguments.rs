use betterlite_models::decisions::Json;
use serde::Serialize;

use super::constants::MAX_PARSED_TOOL_ARGUMENT_CHARS;
use super::values::Object;

// LLM Obs types arguments as an object, or the raw string when they will not parse to one.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ToolArguments {
    Object(Object),
    Text(String),
    // a value outside the declared object-or-string type is carried as it arrived
    Value(Json),
}

// LiteLLM's _to_dd_arguments: an object passes through, a short string is parsed, anything else rides along.
#[must_use]
pub fn to_dd_arguments(value: &Json) -> ToolArguments {
    match value {
        Json::Object(fields) => ToolArguments::Object(fields.clone()),
        Json::Text(text) => decode_tool_arguments(text),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {
            ToolArguments::Value(value.clone())
        }
    }
}

// The size bound runs before the parse, so a hostile compact document is never expanded.
#[must_use]
pub fn decode_tool_arguments(text: &str) -> ToolArguments {
    if text.chars().count() > MAX_PARSED_TOOL_ARGUMENT_CHARS {
        return ToolArguments::Text(text.to_owned());
    }
    match serde_json::from_str::<Json>(text) {
        Ok(Json::Object(fields)) => ToolArguments::Object(fields),
        Ok(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | Err(_) => ToolArguments::Text(text.to_owned()),
    }
}
