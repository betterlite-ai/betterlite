use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::Serialize;

use crate::primitives::json_value::JsonValue;

use super::error::EvaluationError;

pub fn to_json<T>(value: &T) -> Result<JsonValue, EvaluationError>
where
    T: Serialize,
{
    serde_json::from_slice(
        &serde_json::to_vec(value).map_err(|source| EvaluationError::Encode { source })?,
    )
    .map_err(|source| EvaluationError::Encode { source })
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
