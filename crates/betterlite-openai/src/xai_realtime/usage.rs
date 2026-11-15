use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

const RESPONSE_CREATED: &str = "response.created";
const RESPONSE_DONE: &str = "response.done";

#[must_use]
pub fn normalize_response_usage(event: Json, event_type: Option<&str>) -> Json {
    let empty_as_null = match event_type {
        Some(RESPONSE_CREATED) => true,
        Some(RESPONSE_DONE) => false,
        Some(_) | None => return event,
    };
    match event {
        Json::Object(mut fields) => {
            let usage = match fields.get_mut("response") {
                Some(Json::Object(response)) => response.get("usage").cloned(),
                _ => None,
            };
            if let Some(usage) = usage
                && let Some(Json::Object(response)) = fields.get_mut("response")
            {
                response.insert(String::from("usage"), normalize_usage(usage, empty_as_null));
            }
            Json::Object(fields)
        }
        other => other,
    }
}

fn normalize_usage(usage: Json, empty_as_null: bool) -> Json {
    match usage {
        Json::Object(entries) if entries.is_empty() => {
            if empty_as_null {
                Json::Null
            } else {
                default_usage()
            }
        }
        Json::Object(entries) => normalized_usage(entries),
        _ => Json::Null,
    }
}

fn normalized_usage(entries: IndexMap<String, Json>) -> Json {
    let mut normalized = IndexMap::new();
    normalized.insert(
        String::from("total_tokens"),
        entries
            .get("total_tokens")
            .cloned()
            .unwrap_or_else(|| Json::Number(Number::from(0_u8))),
    );
    normalized.insert(
        String::from("input_tokens"),
        entries
            .get("input_tokens")
            .cloned()
            .unwrap_or_else(|| Json::Number(Number::from(0_u8))),
    );
    normalized.insert(
        String::from("output_tokens"),
        entries
            .get("output_tokens")
            .cloned()
            .unwrap_or_else(|| Json::Number(Number::from(0_u8))),
    );
    normalized.insert(
        String::from("input_token_details"),
        token_details(entries.get("input_token_details")),
    );
    normalized.insert(
        String::from("output_token_details"),
        token_details(entries.get("output_token_details")),
    );
    Json::Object(normalized)
}

fn token_details(details: Option<&Json>) -> Json {
    let mut merged = zero_details();
    match details {
        Some(Json::Object(entries)) => {
            for (name, value) in entries {
                merged.insert(name.clone(), value.clone());
            }
        }
        Some(_) | None => {}
    }
    Json::Object(merged)
}

fn default_usage() -> Json {
    Json::Object(IndexMap::from([
        (
            String::from("total_tokens"),
            Json::Number(Number::from(0_u8)),
        ),
        (
            String::from("input_tokens"),
            Json::Number(Number::from(0_u8)),
        ),
        (
            String::from("output_tokens"),
            Json::Number(Number::from(0_u8)),
        ),
        (
            String::from("input_token_details"),
            Json::Object(zero_details()),
        ),
        (
            String::from("output_token_details"),
            Json::Object(zero_details()),
        ),
    ]))
}

fn zero_details() -> IndexMap<String, Json> {
    IndexMap::from([
        (
            String::from("cached_tokens"),
            Json::Number(Number::from(0_u8)),
        ),
        (
            String::from("text_tokens"),
            Json::Number(Number::from(0_u8)),
        ),
        (
            String::from("audio_tokens"),
            Json::Number(Number::from(0_u8)),
        ),
    ])
}
