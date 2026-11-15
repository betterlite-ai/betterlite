use indexmap::IndexMap;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::Serialize;

use crate::primitives::json_value::JsonValue;

use super::error::EvaluationError;

#[must_use]
pub fn object(entries: Vec<(&str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        entries
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect::<IndexMap<_, _>>(),
    )
}

#[must_use]
pub fn array(items: Vec<JsonValue>) -> JsonValue {
    JsonValue::Array(items)
}

#[must_use]
pub fn text(value: &str) -> JsonValue {
    JsonValue::String(value.to_owned())
}

#[must_use]
pub fn integer(value: i64) -> JsonValue {
    JsonValue::Number(serde_json::Number::from(value))
}

pub fn to_json<T>(value: &T) -> Result<JsonValue, EvaluationError>
where
    T: Serialize,
{
    let encoded = serde_json::to_vec(value).map_err(|source| EvaluationError::Encode { source })?;
    serde_json::from_slice(&encoded).map_err(|source| EvaluationError::Encode { source })
}

pub fn header_map(pairs: &[(&str, &str)]) -> Result<HeaderMap, EvaluationError> {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|source| EvaluationError::InvalidHeaderName { source })?;
        let value = HeaderValue::from_str(value)
            .map_err(|source| EvaluationError::InvalidHeader { source })?;
        headers.insert(name, value);
    }
    Ok(headers)
}
