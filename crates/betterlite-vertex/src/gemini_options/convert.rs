use betterlite_prompts::arguments::json::Json;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::error::GeminiOptionsError;

pub fn encode<T>(value: T) -> Result<Json, GeminiOptionsError>
where
    T: Serialize,
{
    serde_json::to_value(value)
        .map_err(GeminiOptionsError::Encode)
        .and_then(|encoded| serde_json::from_value(encoded).map_err(GeminiOptionsError::Encode))
}

pub fn decode<T>(value: &Json, param: &str) -> Result<T, GeminiOptionsError>
where
    T: DeserializeOwned,
{
    let encoded = serde_json::to_value(value).map_err(GeminiOptionsError::Encode)?;
    serde_json::from_value(encoded).map_err(|source| GeminiOptionsError::Invalid {
        param: param.to_owned(),
        source,
    })
}
