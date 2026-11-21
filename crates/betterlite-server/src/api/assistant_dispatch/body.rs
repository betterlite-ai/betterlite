use betterlite_core::api::api_boundaries::{
    ApiBoundariesInput, ApiLimitError, ApiLimits, JsonBoundary,
};
use betterlite_models::decisions::Json;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AssistantBodyError {
    #[error(transparent)]
    Limits(#[from] ApiLimitError),
    #[error("the assistant body is not JSON: {0}")]
    Invalid(#[source] serde_json::Error),
    #[error("the assistant body is not JSON: a number overflows a double")]
    NonFiniteNumber,
}

// orjson.loads at proxy_server.py#L13171: one strict parse, no repair and no object coercion
pub fn decode_assistant_body(bytes: &[u8], limits: &ApiLimits) -> Result<Json, AssistantBodyError> {
    limits.check(ApiBoundariesInput::Json(JsonBoundary {
        known_bytes: bytes.len(),
    }))?;
    let value: Json = serde_json::from_slice(bytes).map_err(AssistantBodyError::Invalid)?;
    finite_floats(&value)?;
    Ok(value)
}

// orjson refuses a float literal whose double is infinite; serde_json keeps the literal text instead
fn finite_floats(value: &Json) -> Result<(), AssistantBodyError> {
    match value {
        Json::Number(number) => {
            let literal = number.to_string();
            // an integer literal of any size stays an integer, as it does in Python
            let float = literal.contains(['.', 'e', 'E']);
            if float && !number.as_f64().is_some_and(f64::is_finite) {
                return Err(AssistantBodyError::NonFiniteNumber);
            }
            Ok(())
        }
        Json::List(items) => items.iter().try_for_each(finite_floats),
        Json::Object(fields) => fields.values().try_for_each(finite_floats),
        Json::Null | Json::Bool(_) | Json::Text(_) => Ok(()),
    }
}
