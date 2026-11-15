use serde_json::{Map, Value};

use super::error::ResponsesChatStreamError;
use crate::api::responses_usage::{UsageInput, is_response_api_usage, normalize_and_project};

// Usage.model_dump() keeps its two declared detail fields, so an absent wrapper still prints as null
pub(super) fn chat_usage(value: &Value) -> Result<Value, ResponsesChatStreamError> {
    let mapped = with_cache_tokens(value);
    let Value::Object(mut fields) = normalize_and_project(UsageInput::Mapping(&mapped))? else {
        return Err(ResponsesChatStreamError::Field { field: "usage" });
    };
    for key in ["prompt_tokens_details", "completion_tokens_details"] {
        fields.entry(key.to_owned()).or_insert(Value::Null);
    }
    Ok(Value::Object(fields))
}

// Usage.__init__ folds the DeepSeek and Anthropic cache counters into the token detail block
fn with_cache_tokens(value: &Value) -> Value {
    let Value::Object(fields) = value else {
        return value.clone();
    };
    let (block, alias) = if is_response_api_usage(fields) {
        ("input_tokens_details", Some("input_token_details"))
    } else {
        ("prompt_tokens_details", None)
    };
    let cached = fields
        .get("cache_read_input_tokens")
        .and_then(Value::as_i64)
        .or_else(|| {
            fields
                .get("prompt_cache_hit_tokens")
                .and_then(Value::as_i64)
        });
    let created = fields
        .get("cache_creation_input_tokens")
        .and_then(Value::as_i64);
    if cached.is_none() && created.is_none() {
        return value.clone();
    }
    let mut details = match fields.get(block) {
        Some(Value::Object(details)) => details.clone(),
        None | Some(Value::Null) => match alias.and_then(|name| fields.get(name)) {
            Some(Value::Object(details)) => details.clone(),
            None | Some(Value::Null) => Map::new(),
            Some(_) => return value.clone(),
        },
        Some(_) => return value.clone(),
    };
    if let Some(tokens) = cached {
        details.insert("cached_tokens".to_owned(), Value::from(tokens));
    }
    if let Some(tokens) = created {
        details.insert("cache_write_tokens".to_owned(), Value::from(tokens));
    }
    let mut mapped = fields.clone();
    mapped.insert(block.to_owned(), Value::Object(details));
    Value::Object(mapped)
}
