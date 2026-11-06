use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::Number;
use std::str::FromStr as _;

use crate::fields::timestamps::with_fraction;
use crate::fields::usage::Usage;

const PROMPT_BASE_KEYS: [&str; 5] = [
    "audio_tokens",
    "cached_tokens",
    "text_tokens",
    "image_tokens",
    "video_tokens",
];

const COMPLETION_KEYS: [&str; 7] = [
    "accepted_prediction_tokens",
    "audio_tokens",
    "reasoning_tokens",
    "rejected_prediction_tokens",
    "text_tokens",
    "image_tokens",
    "video_tokens",
];

const SERVER_KEYS: [&str; 3] = [
    "web_search_requests",
    "tool_search_requests",
    "browser_open_requests",
];

const CACHED_KEYS: [&str; 3] = ["text_tokens", "audio_tokens", "image_tokens"];

const CREATION_KEYS: [&str; 2] = ["ephemeral_5m_input_tokens", "ephemeral_1h_input_tokens"];

#[must_use]
pub fn usage_json(usage: &Usage) -> Json {
    let mut record = IndexMap::new();
    for (name, value) in [
        ("prompt_tokens", usage.prompt_tokens),
        ("completion_tokens", usage.completion_tokens),
        ("total_tokens", usage.total_tokens),
    ] {
        record.insert(name.to_owned(), Json::Number(Number::from(value)));
    }
    record.insert(
        "completion_tokens_details".to_owned(),
        completion_json(usage.completion_tokens_details.as_ref()),
    );
    record.insert(
        "prompt_tokens_details".to_owned(),
        prompt_json(usage.prompt_tokens_details.as_ref()),
    );
    if let Some(server) = &usage.server_tool_use {
        record.insert(
            "server_tool_use".to_owned(),
            keys_json(server, &SERVER_KEYS),
        );
    }
    if let Some(cost) = usage.cost {
        record.insert("cost".to_owned(), decimal_json(cost));
    }
    for (key, value) in &usage.extras {
        record.insert(key.clone(), value.clone());
    }
    Json::Object(record)
}

// PromptTokensDetailsWrapper dumps its five base keys always, a declared field only when set, and unknown provider keys as given.
fn prompt_json(details: Option<&IndexMap<String, Json>>) -> Json {
    let Some(found) = details else {
        return Json::Null;
    };
    let mut record = IndexMap::new();
    for name in PROMPT_BASE_KEYS {
        insert_value_or_null(&mut record, name, found.get(name));
    }
    if let Some(cache_write) = cache_write_value(found) {
        record.insert("cache_write_tokens".to_owned(), cache_write.clone());
        record.insert("cache_creation_tokens".to_owned(), cache_write);
    }
    for (key, value) in found {
        match key.as_str() {
            "audio_tokens"
            | "cached_tokens"
            | "text_tokens"
            | "image_tokens"
            | "video_tokens"
            | "cache_write_tokens"
            | "cache_creation_tokens" => {}
            "cached_tokens_details" => insert_nested(&mut record, key, value, &CACHED_KEYS),
            "cache_creation_token_details" => {
                insert_nested(&mut record, key, value, &CREATION_KEYS);
            }
            "web_search_requests"
            | "google_maps_grounding_requests"
            | "tool_use_tokens"
            | "character_count"
            | "image_count"
            | "video_length_seconds"
            | "audio_length_seconds"
            | "query_count" => insert_set_value(&mut record, key, value),
            _ => {
                record.insert(key.clone(), value.clone());
            }
        }
    }
    Json::Object(record)
}

// The wrapper's __setattr__ mirror: either cache-write key sets both, and a nested cache_creation_input_tokens number counts as the write count.
fn cache_write_value(fields: &IndexMap<String, Json>) -> Option<Json> {
    for name in ["cache_write_tokens", "cache_creation_tokens"] {
        if let Some(value) = fields
            .get(name)
            .filter(|value| !matches!(value, Json::Null))
        {
            return Some(value.clone());
        }
    }
    match fields.get("cache_creation_input_tokens") {
        Some(Json::Number(number)) => Some(Json::Number(number.clone())),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

// CompletionTokensDetailsWrapper dumps all seven declared keys, nulls included, plus its extras.
fn completion_json(details: Option<&IndexMap<String, Json>>) -> Json {
    let Some(found) = details else {
        return Json::Null;
    };
    let mut record = IndexMap::new();
    for name in COMPLETION_KEYS {
        insert_value_or_null(&mut record, name, found.get(name));
    }
    for (key, value) in found {
        if !COMPLETION_KEYS.contains(&key.as_str()) {
            record.insert(key.clone(), value.clone());
        }
    }
    Json::Object(record)
}

// A declared-key record whose dump keeps all its declared keys, nulls included, and drops unknown keys.
fn keys_json(fields: &IndexMap<String, Json>, keys: &[&str]) -> Json {
    let mut record = IndexMap::new();
    for name in keys {
        insert_value_or_null(&mut record, name, fields.get(*name));
    }
    Json::Object(record)
}

// The wrapper's nested detail models project through the same declared-key rule.
fn insert_nested(record: &mut IndexMap<String, Json>, name: &str, value: &Json, keys: &[&str]) {
    if let Json::Object(nested) = value {
        record.insert(name.to_owned(), keys_json(nested, keys));
    }
}

fn insert_value_or_null(record: &mut IndexMap<String, Json>, name: &str, value: Option<&Json>) {
    match value {
        Some(found) => {
            record.insert(name.to_owned(), found.clone());
        }
        None => {
            record.insert(name.to_owned(), Json::Null);
        }
    }
}

fn insert_set_value(record: &mut IndexMap<String, Json>, name: &str, value: &Json) {
    if !matches!(value, Json::Null) {
        record.insert(name.to_owned(), value.clone());
    }
}

// A Decimal always prints as a JSON number, so the parse can only succeed; the null arm keeps the projection total rather than panicking.
fn decimal_json(value: Decimal) -> Json {
    match Number::from_str(&with_fraction(value).to_string()) {
        Ok(number) => Json::Number(number),
        Err(_error) => Json::Null,
    }
}
