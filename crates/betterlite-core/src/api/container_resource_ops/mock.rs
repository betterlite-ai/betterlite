use betterlite_models::decisions::Json;
use serde::de::DeserializeOwned;

use super::error::ContainerOpError;

// the two model names LiteLLM's mock path builds, and the error names the one that did not decode
pub(crate) const RETRIEVE_TYPE: &str = "ContainerObject";
pub(crate) const DELETE_TYPE: &str = "DeleteContainerResult";

// LiteLLM's mock short circuit: a string is JSON-decoded first, then the record is built from the body
pub(crate) fn mock_reply<T>(
    mock: Option<&Json>,
    response_type: &str,
) -> Result<Option<T>, ContainerOpError>
where
    T: DeserializeOwned,
{
    let Some(body) = mock_body(mock)? else {
        return Ok(None);
    };
    decode(&body, response_type).map(Some)
}

// kwargs.get("mock_response") is None for an absent key and for an explicit null, and both skip the mock
fn mock_body(mock: Option<&Json>) -> Result<Option<Json>, ContainerOpError> {
    match mock {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => serde_json::from_str::<Json>(text)
            .map(Some)
            .map_err(|source| ContainerOpError::MockJson { source }),
        Some(value) => Ok(Some(value.clone())),
    }
}

fn decode<T>(body: &Json, response_type: &str) -> Result<T, ContainerOpError>
where
    T: DeserializeOwned,
{
    let text = serde_json::to_string(body).map_err(|source| ContainerOpError::MockModel {
        response_type: response_type.to_owned(),
        source,
    })?;
    serde_json::from_str::<T>(&text).map_err(|source| ContainerOpError::MockModel {
        response_type: response_type.to_owned(),
        source,
    })
}
