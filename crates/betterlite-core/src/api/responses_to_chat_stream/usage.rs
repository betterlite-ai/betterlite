use serde_json::Value;

use super::error::ResponsesChatStreamError;
use crate::api::responses_usage::{UsageInput, normalize_and_project};

// Usage.model_dump() keeps its two declared detail fields, so an absent wrapper still prints as null
pub fn chat_usage(value: &Value) -> Result<Value, ResponsesChatStreamError> {
    let Value::Object(mut fields) = normalize_and_project(UsageInput::Mapping(value))? else {
        return Err(ResponsesChatStreamError::Field { field: "usage" });
    };
    for key in ["prompt_tokens_details", "completion_tokens_details"] {
        fields.entry(key.to_owned()).or_insert(Value::Null);
    }
    Ok(Value::Object(fields))
}
