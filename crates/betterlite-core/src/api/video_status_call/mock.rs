use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::VideoStatusCallError;
use crate::api::video_records::VideoObject;

// LiteLLM's mock short circuit, before the provider decode: a string is parsed as JSON, then VideoObject(**mock_response)
pub(crate) fn mock_response(
    kwargs: &IndexMap<String, Json>,
) -> Result<Option<VideoObject>, VideoStatusCallError> {
    match kwargs.get("mock_response") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => {
            let parsed = serde_json::from_str::<Json>(text)
                .map_err(|source| VideoStatusCallError::MockJson { source })?;
            decode(&parsed).map(Some)
        }
        Some(value) => decode(value).map(Some),
    }
}

fn decode(value: &Json) -> Result<VideoObject, VideoStatusCallError> {
    let text = serde_json::to_string(value)
        .map_err(|source| VideoStatusCallError::MockModel { source })?;
    serde_json::from_str(&text).map_err(|source| VideoStatusCallError::MockModel { source })
}
