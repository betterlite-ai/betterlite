use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink};
use betterlite_models::decisions::{Extra, Json};
use serde_json::Number;

use crate::cache_key::derive::get_cache_key;
use crate::cache_key::value::{KeyObject, KeyValue};
use crate::envelope::facade::ResponseCache;
use crate::types::embedding::{CachedEmbedding, EMBEDDING_CACHE_FORMAT_VERSION, EmbeddingPayload};

use super::HandlerError;
use super::kwargs::{INPUT_KWARG, clear_preset_key, key_request, set_input_param};
use super::value::to_json;

// LiteLLM's handle_kwargs_input_list_or_str: an embedding input is one string or a list of strings and token lists
pub fn input_items(kwargs: &KeyObject) -> Result<Vec<KeyValue>, HandlerError> {
    match kwargs.get(INPUT_KWARG) {
        Some(KeyValue::String(text)) => Ok(vec![KeyValue::String(text.clone())]),
        Some(KeyValue::Array(items)) => {
            if items.iter().all(is_embedding_input) {
                Ok(items.clone())
            } else if is_token_array(items) {
                Ok(vec![KeyValue::Array(items.clone())])
            } else {
                Err(input_error())
            }
        }
        None
        | Some(KeyValue::Null | KeyValue::Bool(_) | KeyValue::Number(_) | KeyValue::Object(_)) => {
            Err(input_error())
        }
    }
}

pub fn embedding_entry_key<L, C>(
    cache: &ResponseCache<L, C>,
    kwargs: &KeyObject,
    input: &KeyValue,
) -> Result<String, HandlerError>
where
    L: LoggerSink,
    C: ConsoleSink,
{
    let mut request = key_request(kwargs)?;
    set_input_param(&mut request, input);
    clear_preset_key(&mut request);
    Ok(get_cache_key(
        &mut request,
        &cache.key_settings,
        &cache.hooks,
    )?)
}

// LiteLLM's _current_format_embedding_entry: every field is required, a wrong shape or an old format version rejects the entry
#[must_use]
pub fn current_format_embedding_entry(entry: &CacheValue) -> Option<CachedEmbedding> {
    let CacheValue::Object(fields) = entry else {
        return None;
    };
    let format_version = match fields.get("format_version")? {
        CacheValue::Integer(version) => *version,
        CacheValue::Null
        | CacheValue::Boolean(_)
        | CacheValue::Float(_)
        | CacheValue::Text(_)
        | CacheValue::Bytes(_)
        | CacheValue::Credential(_)
        | CacheValue::Array(_)
        | CacheValue::Object(_) => return None,
    };
    if format_version != EMBEDDING_CACHE_FORMAT_VERSION {
        return None;
    }
    Some(CachedEmbedding {
        embedding: nullable(fields.get("embedding"), embedding_of).ok()?,
        index: nullable(fields.get("index"), integer_of).ok()?,
        object: nullable(fields.get("object"), text_of).ok()?,
        model: nullable(fields.get("model"), text_of).ok()?,
        prompt_tokens: nullable(fields.get("prompt_tokens"), integer_of).ok()?,
        prompt_tokens_details: nullable(fields.get("prompt_tokens_details"), details_of).ok()?,
        format_version,
    })
}

const fn input_error() -> HandlerError {
    HandlerError::FieldType {
        field: "input",
        expected: "a string or a list",
    }
}

fn is_embedding_input(value: &KeyValue) -> bool {
    match value {
        KeyValue::String(_) => true,
        KeyValue::Array(items) => is_token_array(items),
        KeyValue::Null | KeyValue::Bool(_) | KeyValue::Number(_) | KeyValue::Object(_) => false,
    }
}

fn is_token_array(items: &[KeyValue]) -> bool {
    items
        .iter()
        .all(|item| matches!(item, KeyValue::Number(number) if number.is_i64() || number.is_u64()))
}

struct Rejected;

fn nullable<Value, Extract>(
    value: Option<&CacheValue>,
    extract: Extract,
) -> Result<Option<Value>, Rejected>
where
    Extract: FnOnce(&CacheValue) -> Option<Value>,
{
    let Some(present) = value else {
        return Err(Rejected);
    };
    if matches!(present, CacheValue::Null) {
        return Ok(None);
    }
    extract(present).map(Some).ok_or(Rejected)
}

fn embedding_of(value: &CacheValue) -> Option<EmbeddingPayload> {
    if let CacheValue::Text(text) = value {
        return Some(EmbeddingPayload::Text(text.clone()));
    }
    if let CacheValue::Array(items) = value {
        return embedding_vector(items);
    }
    None
}

const fn integer_of(value: &CacheValue) -> Option<i64> {
    if let CacheValue::Integer(number) = value {
        return Some(*number);
    }
    None
}

fn text_of(value: &CacheValue) -> Option<String> {
    if let CacheValue::Text(text) = value {
        return Some(text.clone());
    }
    None
}

fn embedding_vector(items: &[CacheValue]) -> Option<EmbeddingPayload> {
    let mut numbers = Vec::with_capacity(items.len());
    for item in items {
        numbers.push(embedding_number(item)?);
    }
    Some(EmbeddingPayload::Vector(numbers))
}

fn embedding_number(value: &CacheValue) -> Option<Number> {
    match value {
        CacheValue::Integer(number) => Some(Number::from(*number)),
        CacheValue::Float(number) => Number::from_f64(*number),
        CacheValue::Null
        | CacheValue::Boolean(_)
        | CacheValue::Text(_)
        | CacheValue::Bytes(_)
        | CacheValue::Credential(_)
        | CacheValue::Array(_)
        | CacheValue::Object(_) => None,
    }
}

fn details_of(value: &CacheValue) -> Option<Extra> {
    match to_json(value)? {
        Json::Object(entries) => Some(entries),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}
