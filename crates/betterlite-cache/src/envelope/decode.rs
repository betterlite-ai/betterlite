use betterlite_context::local_ttl_cache::CacheValue;
use thiserror::Error;

use super::{RESPONSE_FIELD, TIMESTAMP_FIELD};
use crate::s3_cache::literal::{self, LiteralError};

#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("the cached envelope timestamp is not a number")]
    Timestamp,
    #[error("the cached envelope has no response")]
    MissingResponse,
    #[error("the cached response is not an object, a string or bytes")]
    ResponseType,
    #[error("the cached value is neither JSON nor a bounded literal")]
    Form {
        #[source]
        source: LiteralError,
    },
    #[error("the cached literal does not fit the cache value model")]
    Convert {
        #[source]
        source: serde_json::Error,
    },
}

// LiteLLM's _get_cache_logic: an envelope with a timestamp is age-checked, and its response is decoded unless it is already a dict.
pub fn decode_cached(
    cached: Option<&CacheValue>,
    max_age: Option<i64>,
    now: f64,
) -> Result<Option<CacheValue>, DecodeError> {
    let Some(cached_value) = cached else {
        return Ok(None);
    };
    let CacheValue::Object(fields) = cached_value else {
        return Ok(Some(cached_value.clone()));
    };
    let Some(stored) = fields.get(TIMESTAMP_FIELD) else {
        return Ok(Some(cached_value.clone()));
    };
    let seconds = timestamp_seconds(stored).ok_or(DecodeError::Timestamp)?;
    // float_arithmetic is denied; a fused multiply-add computes now - seconds
    let age = seconds.mul_add(-1.0_f64, now);
    if let Some(bound) = max_age {
        let limit = serde_json::Number::from(bound).as_f64();
        if limit.is_some_and(|value| age > value) {
            return Ok(None);
        }
    }
    let response = fields
        .get(RESPONSE_FIELD)
        .ok_or(DecodeError::MissingResponse)?;
    decode_response(response).map(Some)
}

// A backend that stores JSON text hands its bytes here; LiteLLM runs json.loads first and ast.literal_eval only when that fails.
pub fn parse_cached_form(bytes: &[u8]) -> Result<CacheValue, DecodeError> {
    if let Ok(value) = serde_json::from_slice(bytes) {
        return Ok(value);
    }
    let literal = literal::parse(bytes).map_err(|source| DecodeError::Form { source })?;
    serde_json::from_value(literal).map_err(|source| DecodeError::Convert { source })
}

fn timestamp_seconds(value: &CacheValue) -> Option<f64> {
    match value {
        CacheValue::Integer(seconds) => serde_json::Number::from(*seconds).as_f64(),
        CacheValue::Float(seconds) => Some(*seconds),
        CacheValue::Null
        | CacheValue::Boolean(_)
        | CacheValue::Text(_)
        | CacheValue::Bytes(_)
        | CacheValue::Array(_)
        | CacheValue::Object(_)
        | CacheValue::Credential(_) => None,
    }
}

fn decode_response(response: &CacheValue) -> Result<CacheValue, DecodeError> {
    match response {
        CacheValue::Object(_) => Ok(response.clone()),
        CacheValue::Text(text) => parse_cached_form(text.as_bytes()),
        CacheValue::Bytes(bytes) => parse_cached_form(bytes),
        CacheValue::Null
        | CacheValue::Boolean(_)
        | CacheValue::Integer(_)
        | CacheValue::Float(_)
        | CacheValue::Array(_)
        | CacheValue::Credential(_) => Err(DecodeError::ResponseType),
    }
}
