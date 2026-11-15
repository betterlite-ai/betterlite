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
    let cached = integer_field(fields, "cache_read_input_tokens")
        .or_else(|| integer_field(fields, "prompt_cache_hit_tokens"));
    let created = integer_field(fields, "cache_creation_input_tokens");
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

fn integer_field(fields: &Map<String, Value>, name: &str) -> Option<i64> {
    fields.get(name)?.as_i64()
}
