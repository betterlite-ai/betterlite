use betterlite_cache::cache_key::value::KeyValue;
use betterlite_cache::embedding_merge::response::{CachedEmbeddingResponse, Embedding};
use betterlite_cache::embedding_merge::usage::Usage;
use betterlite_cache::replay::Replayed;
use betterlite_cache::types::embedding::EmbeddingPayload;
use betterlite_core::caching::lookup::json::{cache_value_json, key_value_json};
use betterlite_core::caching::lookup::response::LookupResponse;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError};
use indexmap::IndexMap;
use serde_json::{Value, json};

use super::record::{HelperResponse, Logged};

pub(crate) fn lookup_report(
    response: Option<&LookupResponse<Replayed<HelperResponse>>>,
    returned: bool,
    logged: &Logged,
) -> Result<Value, Box<HelperError>> {
    let mut rendered = "none";
    let mut raw_value = Value::Null;
    let mut hit_flag: Option<bool> = None;
    let mut stamped_key: Option<String> = None;
    let mut embedding_value = Value::Null;
    let mut every_cached = false;
    let mut uncached = Value::Null;
    if let Some(response) = response {
        if let Some(result) = &response.cached_result {
            match result {
                Replayed::Unchanged(value) => {
                    rendered = "raw";
                    raw_value = serde_json::to_value(cache_value_json(value)).or_value_error()?;
                }
                Replayed::Response(_, converted)
                | Replayed::Transcription(converted)
                | Replayed::Responses(converted)
                | Replayed::Stream(converted)
                | Replayed::ResponsesStream(converted)
                | Replayed::MessagesStream(converted) => {
                    rendered = "converted";
                    hit_flag = Some(converted.cache_hit);
                    stamped_key = converted.stamp.clone().flatten();
                }
            }
        }
        if let Some(reply) = &response.final_embedding_response {
            embedding_value = embedding_report(reply);
        }
        every_cached = response.all_embeddings_cached;
        if let Some(items) = &response.embedding_uncached_input {
            uncached = uncached_report(items)?;
        }
    }
    Ok(json!({
        "returned": returned,
        "cached": rendered,
        "raw": raw_value,
        "cache_hit": hit_flag,
        "cache_key": stamped_key,
        "embedding": embedding_value,
        "all_cached": every_cached,
        "uncached_input": uncached,
        "logging": logging_report(logged),
    }))
}

fn uncached_report(items: &[KeyValue]) -> Result<Value, Box<HelperError>> {
    let rendered = items
        .iter()
        .map(|item| serde_json::to_value(key_value_json(item)))
        .collect::<Result<Vec<_>, _>>()
        .or_value_error()?;
    Ok(Value::Array(rendered))
}

fn embedding_report(response: &CachedEmbeddingResponse) -> Value {
    json!({
        "reply": {
            "model": response.model,
            "data": response.data.iter().map(embedding_item).collect::<Vec<_>>(),
            "usage": response.usage.as_ref().map(usage_report),
        },
        "cache_hit": bool_field(&response.hidden_params, "cache_hit"),
        "cache_key": text_field(&response.hidden_params, "cache_key"),
    })
}

fn embedding_item(slot: &Option<Embedding>) -> Value {
    let Some(item) = slot else {
        return Value::Null;
    };
    json!({
        "embedding": payload_value(&item.embedding),
        "index": item.index,
        "object": "embedding",
    })
}

fn payload_value(payload: &EmbeddingPayload) -> Value {
    match payload {
        EmbeddingPayload::Vector(values) => {
            Value::Array(values.iter().cloned().map(Value::Number).collect())
        }
        EmbeddingPayload::Text(text) => Value::String(text.clone()),
    }
}

fn usage_report(usage: &Usage) -> Value {
    json!({
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
    })
}

fn logging_report(logged: &Logged) -> Value {
    let details = logged.caching_details.as_ref();
    json!({
        "model": logged.model,
        "provider": logged.provider,
        "acompletion": bool_field(&logged.litellm_params, "acompletion"),
        "preset_cache_key": text_field(&logged.litellm_params, "preset_cache_key"),
        "cache_hit": details_bool(details, "cache_hit"),
        "cache_duration": duration_state(details),
    })
}

fn bool_field(params: &IndexMap<String, Json>, name: &str) -> Option<bool> {
    match params.get(name) {
        Some(Json::Bool(flag)) => Some(*flag),
        None | Some(_) => None,
    }
}

fn text_field(params: &IndexMap<String, Json>, name: &str) -> Option<String> {
    match params.get(name) {
        Some(Json::Text(text)) => Some(text.clone()),
        None | Some(_) => None,
    }
}

fn details_bool(details: Option<&Json>, name: &str) -> Option<bool> {
    match details {
        Some(Json::Object(fields)) => bool_field(fields, name),
        None | Some(_) => None,
    }
}

// LiteLLM's CacheDetails record is absent until a hit records it, then its duration is a number or null
fn duration_state(details: Option<&Json>) -> &'static str {
    let Some(Json::Object(fields)) = details else {
        return "absent";
    };
    match fields.get("cache_duration_ms") {
        Some(Json::Null) => "null",
        Some(_) => "number",
        None => "absent",
    }
}
