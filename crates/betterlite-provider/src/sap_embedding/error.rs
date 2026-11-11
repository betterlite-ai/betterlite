use http::HeaderMap;
use serde_json::Error as JsonError;
use thiserror::Error;

use crate::sap_auth::error::SapAuthError;
use crate::sap_models::SapModelError;
use crate::sap_stream::SapOrchestrationError;

#[derive(Debug, Error)]
pub enum SapEmbeddingError {
    #[error(transparent)]
    Auth(#[from] SapAuthError),
    #[error(transparent)]
    Model(#[from] SapModelError),
    #[error("the SAP embedding input must be a string or a list of strings")]
    Input,
    #[error("`version` must be a string")]
    Version,
    #[error("`parameters` must be an object")]
    Parameters,
    #[error("`timeout` must be a whole number of seconds from 1 to 600")]
    Timeout,
    #[error("`max_retries` must be a whole number from 0 to 5")]
    MaxRetries,
    #[error("`type` must be text, document or query")]
    InputType,
    #[error("the SAP embedding request could not be written as JSON")]
    Encode(#[source] JsonError),
    #[error("the SAP embedding request is not the JSON the orchestration service accepts")]
    Request(#[source] JsonError),
    #[error("the SAP embedding response is not the JSON embeddings return")]
    Response(#[source] JsonError),
}

// LiteLLM's GenAIHubEmbeddingConfig.get_error_class keeps the status and message and ignores the headers
#[must_use]
pub fn get_error_class(
    error_message: &str,
    status_code: u16,
    _headers: &HeaderMap,
) -> SapOrchestrationError {
    SapOrchestrationError {
        status_code,
        message: error_message.to_owned(),
    }
}
