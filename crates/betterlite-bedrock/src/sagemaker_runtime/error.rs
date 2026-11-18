use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::chat_content::ChatContentError;
use betterlite_openai::sagemaker_completion::error::SagemakerError;
use betterlite_openai::sagemaker_completion::request::RequestError;
use thiserror::Error;

use crate::credentials::error::CredentialError;
use crate::sagemaker_embedding::cohere::{CohereReplyError, CohereRequestError};
use crate::sagemaker_embedding::hf::HfReplyError;
use crate::signing::error::SigningError;

// every way the runtime call can fail before a reply arrives; the wire failures carry their own SagemakerError
#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error(transparent)]
    Signing(#[from] SigningError),
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error(transparent)]
    Request(#[from] RequestError),
    #[error(transparent)]
    ChatContent(#[from] ChatContentError),
    #[error(transparent)]
    CohereRequest(#[from] CohereRequestError),
    #[error(transparent)]
    CohereReply(#[from] CohereReplyError),
    #[error(transparent)]
    HfReply(#[from] HfReplyError),
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error(transparent)]
    Sagemaker(#[from] SagemakerError),
    #[error("the {field} parameter must be a string")]
    Field { field: &'static str },
    #[error("the {name} header is not a valid HTTP header")]
    Header { name: String },
    #[error("the endpoint {endpoint} is not a valid URL")]
    Endpoint { endpoint: String },
}

impl From<RuntimeError> for SagemakerError {
    fn from(error: RuntimeError) -> Self {
        if let RuntimeError::Sagemaker(inner) = error {
            return inner;
        }
        // LiteLLM reports a failed call that never reached HTTP with a synthesized 500
        let status_code = match &error {
            RuntimeError::HfReply(inner) => inner.status_code(),
            _ => 500,
        };
        Self {
            status_code,
            message: error.to_string(),
            headers: None,
        }
    }
}

#[cfg(test)]
mod tests;
