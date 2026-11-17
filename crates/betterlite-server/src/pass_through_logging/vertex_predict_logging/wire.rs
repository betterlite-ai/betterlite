use betterlite_models::decisions::{Extra, Json};
use serde::Serialize;

use super::VertexPredictError;

// serde_json's own decode names the offending byte, while the JSON tree keeps numbers exact.
pub(super) fn decode_body(body: &[u8]) -> Result<Json, VertexPredictError> {
    serde_json::from_slice(body).map_err(|source| VertexPredictError::Decode { source })
}

pub(super) const fn object_fields(value: &Json) -> Result<&Extra, VertexPredictError> {
    match value {
        Json::Object(fields) => Ok(fields),
        other => Err(VertexPredictError::BodyShape { found: kind(other) }),
    }
}

pub(super) fn json_of<T>(value: &T) -> Result<Json, VertexPredictError>
where
    T: Serialize,
{
    let encoded =
        serde_json::to_value(value).map_err(|source| VertexPredictError::Encode { source })?;
    serde_json::from_value(encoded).map_err(|source| VertexPredictError::Encode { source })
}

const fn kind(value: &Json) -> &'static str {
    match value {
        Json::Null => "null",
        Json::Bool(_) => "a boolean",
        Json::Number(_) => "a number",
        Json::Text(_) => "a string",
        Json::List(_) => "a list",
        Json::Object(_) => "an object",
    }
}
