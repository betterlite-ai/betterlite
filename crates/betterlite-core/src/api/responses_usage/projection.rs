use super::Usage;
use betterlite_models::usage_sum::{CompletionTokensDetails, PromptTokensDetails};
use rust_decimal::Decimal;
use serde_json::{Map, Number, Value};
use std::str::FromStr as _;

pub(super) fn usage_value(usage: &Usage) -> Result<Value, serde_json::Error> {
    let mut result = usage.extensions.clone();
    result.insert("prompt_tokens".to_owned(), Value::from(usage.prompt_tokens));
    result.insert(
        "completion_tokens".to_owned(),
        Value::from(usage.completion_tokens),
    );
    result.insert("total_tokens".to_owned(), Value::from(usage.total_tokens));
    if let Some(details) = &usage.prompt_tokens_details {
        result.insert(
            "prompt_tokens_details".to_owned(),
            prompt_details_value(details),
        );
    }
    if let Some(details) = &usage.completion_tokens_details {
        result.insert(
            "completion_tokens_details".to_owned(),
            completion_details_value(details),
        );
    }
    if let Some(cost) = usage.cost {
        let number = Number::from_str(&cost.normalize().to_string())?;
        result.insert("cost".to_owned(), Value::Number(number));
    }
    Ok(Value::Object(result))
}

pub(super) fn decimal_from_value(value: &Value) -> Option<Decimal> {
    let number = value.as_number()?;
    let decimal_text = number.to_string();
    Decimal::from_str(&decimal_text).ok()
}

fn prompt_details_value(details: &PromptTokensDetails) -> Value {
    let mut fields = Map::new();
    insert_optional(&mut fields, "audio_tokens", details.audio_tokens);
    insert_optional(&mut fields, "cached_tokens", details.cached_tokens);
    insert_optional(&mut fields, "text_tokens", details.text_tokens);
    insert_optional(&mut fields, "image_tokens", details.image_tokens);
    insert_optional(&mut fields, "video_tokens", details.video_tokens);
    insert_optional(
        &mut fields,
        "cache_write_tokens",
        details.cache_write_tokens,
    );
    insert_optional(
        &mut fields,
        "cache_creation_tokens",
        details.cache_creation_tokens,
    );
    insert_optional(
        &mut fields,
        "web_search_requests",
        details.web_search_requests,
    );
    insert_optional(
        &mut fields,
        "google_maps_grounding_requests",
        details.google_maps_grounding_requests,
    );
    if let Some(cached) = &details.cached_tokens_details {
        let mut nested = Map::new();
        insert_optional(&mut nested, "text_tokens", cached.text_tokens);
        insert_optional(&mut nested, "audio_tokens", cached.audio_tokens);
        insert_optional(&mut nested, "image_tokens", cached.image_tokens);
        fields.insert("cached_tokens_details".to_owned(), Value::Object(nested));
    } else {
        fields.insert("cached_tokens_details".to_owned(), Value::Null);
    }
    Value::Object(fields)
}

fn completion_details_value(details: &CompletionTokensDetails) -> Value {
    let mut fields = Map::new();
    insert_optional(&mut fields, "text_tokens", details.text_tokens);
    insert_optional(&mut fields, "reasoning_tokens", details.reasoning_tokens);
    insert_optional(&mut fields, "audio_tokens", details.audio_tokens);
    insert_optional(&mut fields, "image_tokens", details.image_tokens);
    insert_optional(&mut fields, "video_tokens", details.video_tokens);
    Value::Object(fields)
}

fn insert_optional(fields: &mut Map<String, Value>, key: &str, value: Option<i64>) {
    if let Some(counter) = value {
        fields.insert(key.to_owned(), Value::from(counter));
    }
}
