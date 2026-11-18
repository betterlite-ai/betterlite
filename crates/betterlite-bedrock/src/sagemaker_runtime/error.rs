use aws_credential_types::provider::error::CredentialsError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::chat_content::ChatContentError;
use betterlite_openai::sagemaker_completion::error::SagemakerError;
use betterlite_openai::sagemaker_completion::request::RequestError;
use thiserror::Error;

use crate::aws_settings::sources::SourceError;
use crate::credentials::error::CredentialError;
use crate::sagemaker_embedding::cohere::{CohereReplyError, CohereRequestError};
use crate::sagemaker_embedding::hf::HfReplyError;
use crate::signing::error::SigningError;

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
    #[error("the request body cannot be read as JSON: {0}")]
    Json(#[from] serde_json::Error),
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
        // botocore raises NoCredentialsError, which LiteLLM's exception mapping turns into a 400 BadRequest
        let missing = if let RuntimeError::Credential(inner) = &error {
            not_loaded(inner)
        } else if matches!(&error, RuntimeError::Signing(SigningError::NoCredentials)) {
            true
        } else if let RuntimeError::Signing(SigningError::Credential(inner)) = &error {
            not_loaded(inner)
        } else {
            false
        };
        // LiteLLM reports a failed call that never reached HTTP with a synthesized 500
        let status_code = if missing {
            400
        } else if let RuntimeError::HfReply(inner) = &error {
            inner.status_code()
        } else {
            500
        };
        let message = if missing {
            String::from("Unable to locate credentials")
        } else {
            error.to_string()
        };
        Self {
            status_code,
            message,
            headers: None,
        }
    }
}

// make_async_call reports its own timeout as a 408 and lets every other transport failure become a 500
#[must_use]
pub fn timeout_error(error: TransportError) -> SagemakerError {
    if matches!(
        error,
        TransportError::ConnectTimeout(_) | TransportError::Deadline(_)
    ) {
        return SagemakerError {
            status_code: 408,
            message: String::from("Timeout error occurred."),
            headers: None,
        };
    }
    SagemakerError::from(RuntimeError::from(error))
}

// the reply parser runs outside LiteLLM's SagemakerError catch, so its failure carries no HTTP status
#[derive(Debug, Error)]
pub enum RuntimeFailure {
    #[error(transparent)]
    Sagemaker(#[from] SagemakerError),
    #[error("the reply is not JSON: {0}")]
    Reply(#[from] serde_json::Error),
}

impl RuntimeFailure {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Sagemaker(error) => Some(error.status_code),
            Self::Reply(_) => None,
        }
    }
}

impl From<RuntimeError> for RuntimeFailure {
    fn from(error: RuntimeError) -> Self {
        Self::Sagemaker(SagemakerError::from(error))
    }
}

const fn not_loaded(error: &CredentialError) -> bool {
    matches!(
        error,
        CredentialError::MissingCredentials
            | CredentialError::Source(
                SourceError::MissingProvider
                    | SourceError::Provider(CredentialsError::CredentialsNotLoaded(_))
            )
    )
}

mod tests;
