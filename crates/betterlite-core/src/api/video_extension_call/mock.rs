use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::MockDecodeError;
use crate::api::video_records::VideoObject;

// LiteLLM's mock short circuit: a string is parsed as JSON first, then VideoObject(**mock_response)
pub(crate) fn mock_response(
    kwargs: &IndexMap<String, Json>,
) -> Result<Option<VideoObject>, MockDecodeError> {
    match kwargs.get("mock_response") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => {
            let parsed = serde_json::from_str::<Json>(text)
                .map_err(|source| MockDecodeError::Json { source })?;
            decode(&parsed).map(Some)
        }
        Some(value) => decode(value).map(Some),
    }
}

fn decode(value: &Json) -> Result<VideoObject, MockDecodeError> {
    let text = serde_json::to_string(value).map_err(|source| MockDecodeError::Model { source })?;
    serde_json::from_str::<VideoObject>(&text).map_err(|source| MockDecodeError::Model { source })
}
