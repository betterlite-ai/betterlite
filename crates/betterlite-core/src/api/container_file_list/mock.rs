use betterlite_models::decisions::Json;
use betterlite_openai::api::container_contracts::records::ContainerFileListResponse;
use indexmap::IndexMap;

use super::error::MockDecodeError;

// LiteLLM's mock short circuit: a string is parsed as JSON first, then ContainerFileListResponse(**mock_response)
pub(crate) fn mock_response(
    kwargs: &IndexMap<String, Json>,
) -> Result<Option<ContainerFileListResponse>, MockDecodeError> {
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

fn decode(value: &Json) -> Result<ContainerFileListResponse, MockDecodeError> {
    let text = serde_json::to_string(value).map_err(|source| MockDecodeError::Model { source })?;
    serde_json::from_str::<ContainerFileListResponse>(&text)
        .map_err(|source| MockDecodeError::Model { source })
}
