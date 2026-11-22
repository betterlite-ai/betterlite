use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::MockDecodeError;
use crate::api::video_records::VideoObject;

// LiteLLM's mock short circuit: a string is parsed as JSON first, then VideoObject(**mock_response)
pub(crate) fn mock_response(
    kwargs: &IndexMap<String, Json>,
) -> Result<Option<VideoObject>, MockDecodeError> {
    let mock = match kwargs.get("mock_response") {
        None | Some(Json::Null) => return Ok(None),
        Some(Json::Text(text)) => {
            serde_json::from_str::<Json>(text).map_err(|source| MockDecodeError::Json { source })?
        }
        Some(value) => value.clone(),
    };
    let text = serde_json::to_string(&mock).map_err(|source| MockDecodeError::Model { source })?;
    let object = serde_json::from_str::<VideoObject>(&text)
        .map_err(|source| MockDecodeError::Model { source })?;
    Ok(Some(object))
}
