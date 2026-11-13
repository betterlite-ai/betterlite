use betterlite_models::usage_sum::{CompletionTokensDetails, PromptTokensDetails};
use serde_json::{Map, Number, Value};
use std::str::FromStr as _;

use super::error::ResponsesChatStreamError;
use crate::api::responses_usage::{Usage, UsageInput, normalize_usage};

// Usage.model_dump() keeps its declared detail fields, so an absent wrapper still prints as null
pub fn chat_usage(value: &Value) -> Result<Value, ResponsesChatStreamError> {
    let Usage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        prompt_tokens_details,
        completion_tokens_details,
        cost,
        extensions,
    } = normalize_usage(UsageInput::Mapping(value))?;
    let mut fields = extensions;
    fields.insert(
        "prompt_tokens".to_owned(),
        Value::Number(Number::from(prompt_tokens)),
    );
    fields.insert(
        "completion_tokens".to_owned(),
        Value::Number(Number::from(completion_tokens)),
    );
    fields.insert(
        "total_tokens".to_owned(),
        Value::Number(Number::from(total_tokens)),
    );
    fields.insert(
        "prompt_tokens_details".to_owned(),
        prompt_tokens_details
            .as_ref()
            .map_or(Value::Null, prompt_details),
    );
    fields.insert(
        "completion_tokens_details".to_owned(),
        completion_tokens_details
            .as_ref()
            .map_or(Value::Null, completion_details),
    );
    if let Some(amount) = cost {
        let number = Number::from_str(&amount.normalize().to_string()).map_err(|source| {
            ResponsesChatStreamError::Json {
                message: source.to_string(),
            }
        })?;
        fields.insert("cost".to_owned(), Value::Number(number));
    }
    Ok(Value::Object(fields))
}

fn prompt_details(details: &PromptTokensDetails) -> Value {
    let mut fields = Map::new();
    push_nullable(&mut fields, "audio_tokens", details.audio_tokens);
    push_nullable(&mut fields, "cached_tokens", details.cached_tokens);
    push_nullable(&mut fields, "text_tokens", details.text_tokens);
    push_nullable(&mut fields, "image_tokens", details.image_tokens);
    push_nullable(&mut fields, "video_tokens", details.video_tokens);
    push_optional(
        &mut fields,
        "web_search_requests",
        details.web_search_requests,
    );
    push_optional(
        &mut fields,
        "google_maps_grounding_requests",
        details.google_maps_grounding_requests,
    );
    push_optional(
        &mut fields,
        "cache_write_tokens",
        details.cache_write_tokens,
    );
    push_optional(
        &mut fields,
        "cache_creation_tokens",
        details.cache_creation_tokens,
    );
    if let Some(cached) = &details.cached_tokens_details {
        let mut nested = Map::new();
        push_nullable(&mut nested, "text_tokens", cached.text_tokens);
        push_nullable(&mut nested, "audio_tokens", cached.audio_tokens);
        push_nullable(&mut nested, "image_tokens", cached.image_tokens);
        fields.insert("cached_tokens_details".to_owned(), Value::Object(nested));
    }
    Value::Object(fields)
}

fn completion_details(details: &CompletionTokensDetails) -> Value {
    let mut fields = Map::new();
    push_nullable(
        &mut fields,
        "accepted_prediction_tokens",
        details.accepted_prediction_tokens,
    );
    push_nullable(&mut fields, "audio_tokens", details.audio_tokens);
    push_nullable(&mut fields, "reasoning_tokens", details.reasoning_tokens);
    push_nullable(
        &mut fields,
        "rejected_prediction_tokens",
        details.rejected_prediction_tokens,
    );
    push_nullable(&mut fields, "text_tokens", details.text_tokens);
    push_nullable(&mut fields, "image_tokens", details.image_tokens);
    push_nullable(&mut fields, "video_tokens", details.video_tokens);
    Value::Object(fields)
}

// the wrapper deletes the fields it was never given, and keeps these five as null
fn push_nullable(fields: &mut Map<String, Value>, key: &str, count: Option<i64>) {
    match count {
        Some(number) => {
            fields.insert(key.to_owned(), Value::Number(Number::from(number)));
        }
        None => {
            fields.insert(key.to_owned(), Value::Null);
        }
    }
}

fn push_optional(fields: &mut Map<String, Value>, key: &str, count: Option<i64>) {
    if let Some(number) = count {
        fields.insert(key.to_owned(), Value::Number(Number::from(number)));
    }
}
