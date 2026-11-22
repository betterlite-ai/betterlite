use betterlite_models::decisions::Json;
use serde::de::DeserializeOwned;

use super::error::MockDecodeError;

// LiteLLM's mock short circuit: a string is parsed as JSON first, then the typed character is built
pub(crate) fn mock_response<T>(
    mock: Option<&Json>,
    expected: &'static str,
) -> Result<Option<T>, MockDecodeError>
where
    T: DeserializeOwned,
{
    let value = match mock {
        None | Some(Json::Null) => return Ok(None),
        Some(Json::Text(text)) => {
            serde_json::from_str::<Json>(text).map_err(|source| MockDecodeError::Json { source })?
        }
        Some(value) => value.clone(),
    };
    let encoded = serde_json::to_value(&value)
        .map_err(|source| MockDecodeError::Model { expected, source })?;
    serde_json::from_value(encoded)
        .map(Some)
        .map_err(|source| MockDecodeError::Model { expected, source })
}
