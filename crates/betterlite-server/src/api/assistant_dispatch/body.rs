use betterlite_core::api::api_boundaries::{
    ApiBoundariesInput, ApiLimitError, ApiLimits, JsonBoundary,
};
use betterlite_models::decisions::Json;
use serde_json::Number;
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
    let mut value: Json = serde_json::from_slice(bytes).map_err(AssistantBodyError::Invalid)?;
    orjson_numbers(&mut value)?;
    Ok(value)
}

// orjson loads an integer beyond i64..=u64 as a double and refuses any number its double cannot hold
fn orjson_numbers(value: &mut Json) -> Result<(), AssistantBodyError> {
    match value {
        Json::Number(number) => {
            let Some(double) = number.as_f64() else {
                return Err(AssistantBodyError::NonFiniteNumber);
            };
            if number.as_i64().is_none() && number.as_u64().is_none() {
                *number = Number::from_f64(double).ok_or(AssistantBodyError::NonFiniteNumber)?;
            }
            Ok(())
        }
        Json::List(items) => items.iter_mut().try_for_each(orjson_numbers),
        Json::Object(fields) => fields.values_mut().try_for_each(orjson_numbers),
        Json::Null | Json::Bool(_) | Json::Text(_) => Ok(()),
    }
}
