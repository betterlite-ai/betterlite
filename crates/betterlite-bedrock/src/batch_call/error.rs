use betterlite_logging::logging_pre_post::error::PrePostError;
use thiserror::Error;

use crate::aws_endpoints::error::AwsEndpointError;
use crate::aws_settings::AuthError;
use crate::batch_status::error::BatchStatusError;
use crate::bedrock_embedding_io::transport::TransferError;
use crate::credentials::error::CredentialError;
use crate::embedding_jobs::error::{EmbeddingJobsError, FailureBody};
use crate::signing::error::SigningError;

#[derive(Debug, Error)]
pub enum BatchCallError {
    #[error(transparent)]
    Credentials(#[from] CredentialError),
    #[error(transparent)]
    Status(#[from] BatchStatusError),
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error(transparent)]
    Region(#[from] AwsEndpointError),
    #[error(transparent)]
    Signing(#[from] SigningError),
    #[error(transparent)]
    Transport(#[from] TransferError),
    #[error(transparent)]
    Logging(#[from] PrePostError),
    #[error(transparent)]
    Async(#[from] EmbeddingJobsError),
    #[error("the Bedrock control plane reply is not JSON")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Bedrock control plane reply is not a JSON object")]
    ResponseShape,
    #[error("the {field} field is missing from the Bedrock control plane reply")]
    MissingField { field: &'static str },
    #[error("the {field} field has the wrong type")]
    Field { field: &'static str },
    #[error("the {status} value is not a batch status")]
    InvalidStatus { status: String },
    #[error("the Bedrock control plane call failed with status {status}")]
    Client {
        status: u16,
        code: Option<String>,
        body: FailureBody,
    },
}
