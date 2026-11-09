use crate::private_json::error::PrivateJsonError;

#[derive(Debug, thiserror::Error)]
pub enum DatadogPayloadError {
    #[error(transparent)]
    Json(#[from] PrivateJsonError),
    #[error("the legacy response object is not a JSON object")]
    ResponseNotObject,
    #[error("the legacy response object's id is not a string")]
    ResponseIdNotText,
    #[error("the legacy response object has no usage")]
    MissingUsage,
    #[error("the legacy response object's usage is not a JSON object")]
    UsageNotObject,
}
