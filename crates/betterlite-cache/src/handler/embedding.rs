use std::collections::BTreeMap;

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
        embedding: nullable_embedding(fields).ok()?,
        index: nullable_integer(fields, "index").ok()?,
        object: nullable_string(fields, "object").ok()?,
        model: nullable_string(fields, "model").ok()?,
        prompt_tokens: nullable_integer(fields, "prompt_tokens").ok()?,
        prompt_tokens_details: nullable_details(fields).ok()?,
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
        KeyValue::Array(items) => items.iter().all(
            |item| matches!(item, KeyValue::Number(number) if number.is_i64() || number.is_u64()),
        ),
        KeyValue::Null | KeyValue::Bool(_) | KeyValue::Number(_) | KeyValue::Object(_) => false,
    }
}

struct Rejected;

fn nullable_embedding(
    fields: &BTreeMap<String, CacheValue>,
) -> Result<Option<EmbeddingPayload>, Rejected> {
    match fields.get("embedding").ok_or(Rejected)? {
        CacheValue::Null => Ok(None),
        CacheValue::Text(text) => Ok(Some(EmbeddingPayload::Text(text.clone()))),
        CacheValue::Array(items) => {
            let mut numbers = Vec::with_capacity(items.len());
            for item in items {
                numbers.push(embedding_number(item)?);
            }
            Ok(Some(EmbeddingPayload::Vector(numbers)))
        }
        CacheValue::Boolean(_)
        | CacheValue::Integer(_)
        | CacheValue::Float(_)
        | CacheValue::Bytes(_)
        | CacheValue::Credential(_)
        | CacheValue::Object(_) => Err(Rejected),
    }
}

fn embedding_number(value: &CacheValue) -> Result<Number, Rejected> {
    match value {
        CacheValue::Integer(number) => Ok(Number::from(*number)),
        CacheValue::Float(number) => Number::from_f64(*number).ok_or(Rejected),
        CacheValue::Null
        | CacheValue::Boolean(_)
        | CacheValue::Text(_)
        | CacheValue::Bytes(_)
        | CacheValue::Credential(_)
        | CacheValue::Array(_)
        | CacheValue::Object(_) => Err(Rejected),
    }
}

fn nullable_integer(
    fields: &BTreeMap<String, CacheValue>,
    name: &str,
) -> Result<Option<i64>, Rejected> {
    match fields.get(name).ok_or(Rejected)? {
        CacheValue::Null => Ok(None),
        CacheValue::Integer(number) => Ok(Some(*number)),
        CacheValue::Boolean(_)
        | CacheValue::Float(_)
        | CacheValue::Text(_)
        | CacheValue::Bytes(_)
        | CacheValue::Credential(_)
        | CacheValue::Array(_)
        | CacheValue::Object(_) => Err(Rejected),
    }
}

fn nullable_string(
    fields: &BTreeMap<String, CacheValue>,
    name: &str,
) -> Result<Option<String>, Rejected> {
    match fields.get(name).ok_or(Rejected)? {
        CacheValue::Null => Ok(None),
        CacheValue::Text(text) => Ok(Some(text.clone())),
        CacheValue::Boolean(_)
        | CacheValue::Integer(_)
        | CacheValue::Float(_)
        | CacheValue::Bytes(_)
        | CacheValue::Credential(_)
        | CacheValue::Array(_)
        | CacheValue::Object(_) => Err(Rejected),
    }
}

fn nullable_details(fields: &BTreeMap<String, CacheValue>) -> Result<Option<Extra>, Rejected> {
    match fields.get("prompt_tokens_details").ok_or(Rejected)? {
        CacheValue::Null => Ok(None),
        value @ CacheValue::Object(_) => match to_json(value).ok_or(Rejected)? {
            Json::Object(entries) => Ok(Some(entries)),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                Err(Rejected)
            }
        },
        CacheValue::Boolean(_)
        | CacheValue::Integer(_)
        | CacheValue::Float(_)
        | CacheValue::Text(_)
        | CacheValue::Bytes(_)
        | CacheValue::Credential(_)
        | CacheValue::Array(_) => Err(Rejected),
    }
}
