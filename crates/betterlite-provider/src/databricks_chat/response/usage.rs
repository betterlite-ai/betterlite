use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::Number;

use crate::databricks_chat::error::DatabricksChatError;
use crate::databricks_chat::response::Usage;

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
    let completion = count(block.get("completion_tokens"), "completion_tokens")?;
    let mut completion_details = object_of(block.get("completion_tokens_details"));
    if let Some(reasoning) = present_count(block.get("reasoning_tokens"), "reasoning_tokens")? {
        let fields = completion_details.get_or_insert_with(Object::new);
        if matches!(fields.get("reasoning_tokens"), None | Some(Json::Null)) {
            fields.insert(
                "reasoning_tokens".to_owned(),
                Json::Number(Number::from(reasoning)),
            );
        }
        if let Some(completion) = completion
            && matches!(fields.get("text_tokens"), None | Some(Json::Null))
        {
            let mut text = completion.saturating_sub(reasoning);
            for name in ["image_tokens", "audio_tokens"] {
                text = text.saturating_sub(present_count(fields.get(name), name)?.unwrap_or(0));
            }
            fields.insert(
                "text_tokens".to_owned(),
                Json::Number(Number::from(text.max(0))),
            );
        }
    }
    Ok(Usage {
        completion_tokens: completion.unwrap_or(0),
        prompt_tokens: count(block.get("prompt_tokens"), "prompt_tokens")?.unwrap_or(0),
        total_tokens: count(block.get("total_tokens"), "total_tokens")?.unwrap_or(0),
        completion_tokens_details: completion_details,
        prompt_tokens_details: prompt_details(block),
        extra: extras(block),
    })
}

// prompt_tokens or 0: an absent or null count is zero, any other type is a shape error
fn count(value: Option<&Json>, name: &str) -> Result<Option<i64>, DatabricksChatError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Number(number)) if number.as_i64().is_some() => Ok(number.as_i64()),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(DatabricksChatError::Shape {
                message: format!("the {name} of the Databricks usage must be an integer"),
            })
        }
    }
}

// if reasoning_tokens: a falsy value counts as absent, any other value must be an integer
fn present_count(value: Option<&Json>, name: &str) -> Result<Option<i64>, DatabricksChatError> {
    if value.is_some_and(|given| !given.is_truthy()) {
        return Ok(None);
    }
    count(value, name)
}

fn object_of(value: Option<&Json>) -> Option<Object> {
    value
        .and_then(Json::as_object)
        .filter(|fields| !fields.is_empty())
        .cloned()
}

// prompt_tokens_details, then the prompt cache keys Usage folds into it
fn prompt_details(block: &Object) -> Option<Object> {
    let base = object_of(block.get("prompt_tokens_details"));
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
        let Some(value) = count(block.get(source), source).unwrap_or_default() else {
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
    let write = count(fields.get("cache_write_tokens"), "cache_write_tokens").unwrap_or_default();
    if write.is_none() {
        let creation = count(fields.get("cache_creation_tokens"), "cache_creation_tokens")
            .unwrap_or_default()
            .or_else(|| {
                count(
                    fields.get("cache_creation_input_tokens"),
                    "cache_creation_input_tokens",
                )
                .unwrap_or_default()
            });
        if let Some(value) = creation {
            fields.insert(
                "cache_write_tokens".to_owned(),
                Json::Number(Number::from(value)),
            );
        }
    }
    if let Some(value) =
        count(fields.get("cache_write_tokens"), "cache_write_tokens").unwrap_or_default()
    {
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
    Json::Object(
        SERVER_TOOL_USE
            .iter()
            .filter_map(|name| {
                fields
                    .get(*name)
                    .map(|entry| ((*name).to_owned(), entry.clone()))
            })
            .collect(),
    )
}
