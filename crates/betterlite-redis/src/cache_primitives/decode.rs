mod tests;

use std::collections::BTreeMap;
use std::num::ParseIntError;
use std::str;
use std::str::Utf8Error;

use serde::Deserialize;
use thiserror::Error;

pub const INCREMENT_WITH_FLOOR_LUA: &str = "local count = redis.call('INCRBY', KEYS[1], ARGV[1]) if count < 0 then count = redis.call('INCRBY', KEYS[1], -count) end if redis.call('TTL', KEYS[1]) < 0 then redis.call('EXPIRE', KEYS[1], ARGV[2]) end return count";

// serde_json::Value is disallowed (clippy.toml); mirrors McpValue's shape for the same reason.
#[derive(Debug, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum CachedValue {
    Object(BTreeMap<String, Self>),
    Null,
    Boolean(bool),
    Number(serde_json::Number),
    Text(String),
    Array(Vec<Self>),
}

#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("redis reply was not valid UTF-8")]
    Utf8 {
        #[source]
        source: Utf8Error,
    },
    #[error("\"{text}\" is not a valid integer count")]
    Count {
        text: String,
        #[source]
        source: ParseIntError,
    },
    #[error("\"{text}\" is not valid JSON or a Python literal")]
    LiteralEval {
        text: String,
        #[source]
        source: py_literal::ParseError,
    },
    #[error("the decoded literal has no JSON representation")]
    UnsupportedLiteral,
    #[error("the decoded literal contains an integer outside i64 range")]
    IntegerOutOfRange,
    #[error("the decoded literal contains a non-finite float")]
    NonFiniteFloat,
    #[error("the decoded literal contains a non-string dict key")]
    NonStringKey,
}

pub fn decoded_counts(values: &[Option<Vec<u8>>]) -> Result<Vec<Option<i64>>, DecodeError> {
    let mut counts = Vec::with_capacity(values.len());
    for value in values {
        counts.push(decode_count(value.as_deref())?);
    }
    Ok(counts)
}

pub fn validate_lua_count(reply: &[u8]) -> Result<i64, DecodeError> {
    let text = str::from_utf8(reply).map_err(|source| DecodeError::Utf8 { source })?;
    text.parse().map_err(|source| DecodeError::Count {
        text: text.to_owned(),
        source,
    })
}

fn decode_count(reply: Option<&[u8]>) -> Result<Option<i64>, DecodeError> {
    let Some(bytes) = reply else { return Ok(None) };
    validate_lua_count(bytes).map(Some)
}

// a value that fails both json and literal-eval propagates the literal-eval error, never None
pub fn decode_cached_value(
    cached_response: Option<&[u8]>,
) -> Result<Option<CachedValue>, DecodeError> {
    let Some(bytes) = cached_response else {
        return Ok(None);
    };
    let decoded = str::from_utf8(bytes).map_err(|source| DecodeError::Utf8 { source })?;
    if let Ok(value) = serde_json::from_str(decoded) {
        return Ok(Some(value));
    }
    let literal =
        decoded
            .parse::<py_literal::Value>()
            .map_err(|source| DecodeError::LiteralEval {
                text: decoded.to_owned(),
                source,
            })?;
    literal_to_cached_value(literal).map(Some)
}

fn literal_to_cached_value(literal: py_literal::Value) -> Result<CachedValue, DecodeError> {
    match literal {
        py_literal::Value::None => Ok(CachedValue::Null),
        py_literal::Value::Boolean(flag) => Ok(CachedValue::Boolean(flag)),
        py_literal::Value::String(text) => Ok(CachedValue::Text(text)),
        py_literal::Value::Integer(integer) => {
            let bounded =
                i64::try_from(&integer).map_err(|_source| DecodeError::IntegerOutOfRange)?;
            Ok(CachedValue::Number(serde_json::Number::from(bounded)))
        }
        py_literal::Value::Float(float) => serde_json::Number::from_f64(float)
            .map(CachedValue::Number)
            .ok_or(DecodeError::NonFiniteFloat),
        py_literal::Value::List(items)
        | py_literal::Value::Tuple(items)
        | py_literal::Value::Set(items) => {
            let elements = items
                .into_iter()
                .map(literal_to_cached_value)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(CachedValue::Array(elements))
        }
        py_literal::Value::Dict(entries) => {
            let mut object = BTreeMap::new();
            for (raw_key, raw_value) in entries {
                let py_literal::Value::String(key) = raw_key else {
                    return Err(DecodeError::NonStringKey);
                };
                object.insert(key, literal_to_cached_value(raw_value)?);
            }
            Ok(CachedValue::Object(object))
        }
        py_literal::Value::Bytes(_) | py_literal::Value::Complex(_) => {
            Err(DecodeError::UnsupportedLiteral)
        }
    }
}
