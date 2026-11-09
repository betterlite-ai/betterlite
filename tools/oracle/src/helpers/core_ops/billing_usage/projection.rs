use betterlite_logging::fields::usage::Usage;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::{Map, Value};
use std::str::FromStr as _;

pub(super) fn usage_projection(usage: Option<&Usage>) -> Value {
    let Some(found_usage) = usage else {
        return Value::Null;
    };
    let mut record = Map::new();
    record.insert(
        "prompt_tokens".to_owned(),
        Value::from(found_usage.prompt_tokens),
    );
    record.insert(
        "completion_tokens".to_owned(),
        Value::from(found_usage.completion_tokens),
    );
    record.insert(
        "total_tokens".to_owned(),
        Value::from(found_usage.total_tokens),
    );
    if let Some(details) = found_usage.prompt_tokens_details.as_ref() {
        record.insert(
            "prompt_tokens_details".to_owned(),
            details_projection(details, &PROMPT_FIELDS),
        );
    }
    if let Some(details) = found_usage.completion_tokens_details.as_ref() {
        record.insert(
            "completion_tokens_details".to_owned(),
            details_projection(details, &COMPLETION_FIELDS),
        );
    }
    if let Some(cost) = found_usage.cost {
        record.insert("cost".to_owned(), decimal_value(&cost));
    }
    record.insert(
        "extras".to_owned(),
        serde_json::to_value(&found_usage.extras).unwrap_or(Value::Null),
    );
    Value::Object(record)
}

const PROMPT_FIELDS: [&str; 6] = [
    "text_tokens",
    "audio_tokens",
    "image_tokens",
    "video_tokens",
    "cached_tokens",
    "cache_write_tokens",
];
const COMPLETION_FIELDS: [&str; 5] = [
    "text_tokens",
    "audio_tokens",
    "image_tokens",
    "video_tokens",
    "reasoning_tokens",
];

fn details_projection(details: &IndexMap<String, Json>, fields: &[&str]) -> Value {
    let mut record = Map::new();
    for name in fields {
        if let Some(value) = details.get(*name) {
            record.insert(
                (*name).to_owned(),
                serde_json::to_value(value).unwrap_or(Value::Null),
            );
        }
    }
    Value::Object(record)
}

pub(super) fn decimal_value(cost: &Decimal) -> Value {
    let text = cost.normalize().to_string();
    serde_json::Number::from_str(&text)
        .map_or_else(|_error| Value::String(cost.to_string()), Value::Number)
}

pub(super) fn optional_text(value: Option<&str>) -> Value {
    match value {
        Some(text) => Value::String(text.to_owned()),
        None => Value::Null,
    }
}
