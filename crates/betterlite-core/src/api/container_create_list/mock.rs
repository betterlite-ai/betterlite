use betterlite_models::decisions::Json;
use serde::de::DeserializeOwned;

use super::error::MockDecodeError;

// LiteLLM's mock short circuit: a string is parsed as JSON first, then the typed response is built
pub(crate) fn mock_response<T>(
    mock: Option<&Json>,
    expected: &'static str,
) -> Result<Option<T>, MockDecodeError>
where
    T: DeserializeOwned,
{
    match mock {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => {
            let parsed = serde_json::from_str::<Json>(text)
                .map_err(|source| MockDecodeError::Json { source })?;
            decode(&parsed)
                .map(Some)
                .map_err(|source| MockDecodeError::Model { expected, source })
        }
        Some(value) => decode(value)
            .map(Some)
            .map_err(|source| MockDecodeError::Model { expected, source }),
    }
}

fn decode<T>(value: &Json) -> Result<T, serde_json::Error>
where
    T: DeserializeOwned,
{
    serde_json::to_value(value).and_then(serde_json::from_value)
}
