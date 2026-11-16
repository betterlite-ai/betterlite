use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::Number;

use crate::databricks_chat::error::DatabricksChatError;
use crate::databricks_chat::response::reply::Usage;

// Usage(**block) consumes these kwargs; every other key is forwarded onto the usage object
const NAMED: [&str; 8] = [
    "prompt_tokens",
    "completion_tokens",
    "total_tokens",
    "reasoning_tokens",
    "prompt_tokens_details",
    "completion_tokens_details",
    "server_tool_use",
    "cost",
];

// PromptTokensDetailsWrapper deletes these when they stay unset
const PROMPT_DROPPED: [&str; 12] = [
    "character_count",
    "image_count",
    "video_length_seconds",
    "audio_length_seconds",
    "query_count",
    "web_search_requests",
    "google_maps_grounding_requests",
    "tool_use_tokens",
    "cache_write_tokens",
    "cache_creation_tokens",
    "cache_creation_token_details",
    "cached_tokens_details",
];

const SERVER_TOOL_USE: [&str; 3] = [
    "web_search_requests",
    "tool_search_requests",
    "browser_open_requests",
];

// usage_from folds the counts, the reasoning details, the prompt cache keys and the forwarded keys
pub(crate) fn usage_from(block: &Object) -> Result<Usage, DatabricksChatError> {
    let prompt_tokens = count(block, "prompt_tokens")?.unwrap_or(0);
    let completion = count(block, "completion_tokens")?;
    let total_tokens = count(block, "total_tokens")?.unwrap_or(0);
    let mut completion_details = truthy_object(block.get("completion_tokens_details"));
    apply_reasoning(block, completion, &mut completion_details)?;
    let prompt_details = prompt_details(block);
    Ok(Usage {
        completion_tokens: completion.unwrap_or(0),
        prompt_tokens,
        total_tokens,
        completion_tokens_details: completion_details,
        prompt_tokens_details: prompt_details,
        extra: extras(block),
    })
}

// prompt_tokens or 0: an absent or null count is zero, any other type is a shape error
fn count(block: &Object, name: &str) -> Result<Option<i64>, DatabricksChatError> {
    match block.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Number(number)) => match number.as_i64() {
            Some(value) => Ok(Some(value)),
            None => Err(wrong_type(name, "an integer")),
        },
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(wrong_type(name, "an integer"))
        }
    }
}

fn truthy_object(value: Option<&Json>) -> Option<Object> {
    match value {
        Some(Json::Object(fields)) if !fields.is_empty() => Some(fields.clone()),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | Some(Json::Object(_)) => None,
    }
}

// if reasoning_tokens: it fills completion_tokens_details and derives text_tokens
fn apply_reasoning(
    block: &Object,
    completion: Option<i64>,
    details: &mut Option<Object>,
) -> Result<(), DatabricksChatError> {
    let Some(reasoning) = truthy_integer(block.get("reasoning_tokens"), "reasoning_tokens")? else {
        return Ok(());
    };
    let fields = details.get_or_insert_with(Object::new);
    if matches!(fields.get("reasoning_tokens"), None | Some(Json::Null)) {
        fields.insert(
            "reasoning_tokens".to_owned(),
            Json::Number(Number::from(reasoning)),
        );
    }
    let Some(completion) = completion else {
        return Ok(());
    };
    if !matches!(fields.get("text_tokens"), None | Some(Json::Null)) {
        return Ok(());
    }
    let mut text = completion.saturating_sub(reasoning);
    for name in ["image_tokens", "audio_tokens"] {
        if let Some(value) = truthy_integer(fields.get(name), name)? {
            text = text.saturating_sub(value);
        }
    }
    fields.insert(
        "text_tokens".to_owned(),
        Json::Number(Number::from(text.max(0))),
    );
    Ok(())
}

// Python treats a falsy value as absent; anything truthy beyond an integer is a shape error
fn truthy_integer(value: Option<&Json>, name: &str) -> Result<Option<i64>, DatabricksChatError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if !value.is_truthy() {
        return Ok(None);
    }
    match value {
        Json::Number(number) => match number.as_i64() {
            Some(count) => Ok(Some(count)),
            None => Err(wrong_type(name, "an integer")),
        },
        Json::Text(_) | Json::Bool(_) | Json::List(_) | Json::Object(_) => {
            Err(wrong_type(name, "an integer"))
        }
    }
}

// prompt_tokens_details, then the prompt cache keys Usage folds into it
fn prompt_details(block: &Object) -> Option<Object> {
    let base = truthy_object(block.get("prompt_tokens_details"));
    let present = base.is_some();
    let mut fields = base.unwrap_or_default();
    if present {
        resolve_cache_write(&mut fields);
        for name in PROMPT_DROPPED {
            if matches!(fields.get(name), None | Some(Json::Null)) {
                fields.shift_remove(name);
            }
        }
    }
    for (source, target) in [
        ("prompt_cache_hit_tokens", "cached_tokens"),
        ("cache_read_input_tokens", "cached_tokens"),
        ("cache_creation_input_tokens", "cache_write_tokens"),
    ] {
        let Some(value) = integer_value(block.get(source)) else {
            continue;
        };
        fields.insert(target.to_owned(), Json::Number(Number::from(value)));
        if target == "cache_write_tokens" {
            fields.insert(
                "cache_creation_tokens".to_owned(),
                Json::Number(Number::from(value)),
            );
        }
    }
    if !present && fields.is_empty() {
        return None;
    }
    Some(fields)
}

// PromptTokensDetailsWrapper mirrors cache_write_tokens and cache_creation_tokens onto each other
fn resolve_cache_write(fields: &mut Object) {
    if integer_value(fields.get("cache_write_tokens")).is_none() {
        let creation = integer_value(fields.get("cache_creation_tokens"))
            .or_else(|| integer_value(fields.get("cache_creation_input_tokens")));
        if let Some(value) = creation {
            fields.insert(
                "cache_write_tokens".to_owned(),
                Json::Number(Number::from(value)),
            );
        }
    }
    if let Some(value) = integer_value(fields.get("cache_write_tokens")) {
        fields.insert(
            "cache_creation_tokens".to_owned(),
            Json::Number(Number::from(value)),
        );
    }
}

fn extras(block: &Object) -> Object {
    let mut extra = block.clone();
    for name in NAMED {
        extra.shift_remove(name);
    }
    if let Some(server_tool_use) = block.get("server_tool_use")
        && !matches!(server_tool_use, Json::Null)
    {
        extra.insert(
            "server_tool_use".to_owned(),
            server_tool_use_fields(server_tool_use),
        );
    }
    if let Some(cost) = block.get("cost")
        && !matches!(cost, Json::Null)
    {
        extra.insert("cost".to_owned(), cost.clone());
    }
    extra
}

// ServerToolUse(**dict) keeps only its three declared fields
fn server_tool_use_fields(value: &Json) -> Json {
    let Json::Object(fields) = value else {
        return value.clone();
    };
    let mut kept = Object::new();
    for name in SERVER_TOOL_USE {
        if let Some(entry) = fields.get(name) {
            kept.insert(name.to_owned(), entry.clone());
        }
    }
    Json::Object(kept)
}

fn integer_value(value: Option<&Json>) -> Option<i64> {
    match value {
        Some(Json::Number(number)) => number.as_i64(),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            None
        }
    }
}

fn wrong_type(name: &str, expected: &str) -> DatabricksChatError {
    DatabricksChatError::Shape {
        message: format!("the {name} of the Databricks usage must be {expected}"),
    }
}
