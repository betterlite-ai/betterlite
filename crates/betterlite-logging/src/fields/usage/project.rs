use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::Number;
use std::str::FromStr as _;

use crate::fields::timestamps::with_fraction;
use crate::fields::usage::Usage;

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
            keys_json(
                server,
                &[
                    "web_search_requests",
                    "tool_search_requests",
                    "browser_open_requests",
                ],
            ),
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

fn prompt_json(details: Option<&IndexMap<String, Json>>) -> Json {
    let Some(found) = details else {
        return Json::Null;
    };
    let mut record = IndexMap::new();
    insert_prompt_base(&mut record, found);
    insert_prompt_fields(&mut record, found);
    Json::Object(record)
}

fn insert_prompt_base(record: &mut IndexMap<String, Json>, found: &IndexMap<String, Json>) {
    insert_value_or_null(record, "audio_tokens", found.get("audio_tokens"));
    insert_value_or_null(record, "cached_tokens", found.get("cached_tokens"));
    insert_value_or_null(record, "text_tokens", found.get("text_tokens"));
    insert_value_or_null(record, "image_tokens", found.get("image_tokens"));
    insert_value_or_null(record, "video_tokens", found.get("video_tokens"));
    if let Some(cache_write) = cache_write_value(found) {
        record.insert("cache_write_tokens".to_owned(), cache_write.clone());
        record.insert("cache_creation_tokens".to_owned(), cache_write);
    }
}

fn insert_prompt_fields(record: &mut IndexMap<String, Json>, found: &IndexMap<String, Json>) {
    for (key, value) in found {
        match key.as_str() {
            "cache_write_tokens" | "cache_creation_tokens" => {}
            "cached_tokens_details" => insert_nested(
                record,
                key,
                value,
                &["text_tokens", "audio_tokens", "image_tokens"],
            ),
            "cache_creation_token_details" => {
                insert_nested(
                    record,
                    key,
                    value,
                    &["ephemeral_5m_input_tokens", "ephemeral_1h_input_tokens"],
                );
            }
            "web_search_requests"
            | "google_maps_grounding_requests"
            | "tool_use_tokens"
            | "character_count"
            | "image_count"
            | "video_length_seconds"
            | "audio_length_seconds"
            | "query_count" => {
                if !matches!(value, Json::Null) {
                    record.insert(key.clone(), value.clone());
                }
            }
            _ => {
                record.insert(key.clone(), value.clone());
            }
        }
    }
}

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

fn completion_json(details: Option<&IndexMap<String, Json>>) -> Json {
    let Some(found) = details else {
        return Json::Null;
    };
    let mut record = IndexMap::new();
    insert_value_or_null(
        &mut record,
        "accepted_prediction_tokens",
        found.get("accepted_prediction_tokens"),
    );
    insert_value_or_null(&mut record, "audio_tokens", found.get("audio_tokens"));
    insert_value_or_null(
        &mut record,
        "reasoning_tokens",
        found.get("reasoning_tokens"),
    );
    insert_value_or_null(
        &mut record,
        "rejected_prediction_tokens",
        found.get("rejected_prediction_tokens"),
    );
    insert_value_or_null(&mut record, "text_tokens", found.get("text_tokens"));
    insert_value_or_null(&mut record, "image_tokens", found.get("image_tokens"));
    insert_value_or_null(&mut record, "video_tokens", found.get("video_tokens"));
    for (key, value) in found {
        if !record.contains_key(key) {
            record.insert(key.clone(), value.clone());
        }
    }
    Json::Object(record)
}

fn keys_json(fields: &IndexMap<String, Json>, keys: &[&str]) -> Json {
    let mut record = IndexMap::new();
    for name in keys {
        insert_value_or_null(&mut record, name, fields.get(*name));
    }
    Json::Object(record)
}

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

fn decimal_json(value: Decimal) -> Json {
    match Number::from_str(&with_fraction(value).to_string()) {
        Ok(number) => Json::Number(number),
        Err(_error) => Json::Null,
    }
}
