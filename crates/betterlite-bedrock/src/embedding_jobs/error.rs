use std::fmt::{self, Debug, Formatter};

use betterlite_logging::logging_pre_post::error::PrePostError;
use thiserror::Error;

use crate::bedrock_embedding_io::error::EmbeddingIoError;
use crate::bedrock_embedding_io::transport::TransferError;
use crate::signing::error::SigningError;

// A provider reply can echo credentials, so Debug names the failure and hides the text
#[derive(Clone, PartialEq, Eq)]
pub struct FailureBody(pub String);

impl Debug for FailureBody {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED provider reply]")
    }
}

#[derive(Debug, Error)]
pub enum EmbeddingJobsError {
    #[error(transparent)]
    Credentials(#[from] EmbeddingIoError),
    #[error("the async invoke status request needs credentials")]
    MissingCredentials,
    #[error(transparent)]
    Signing(#[from] SigningError),
    #[error(transparent)]
    Transport(#[from] TransferError),
    #[error(transparent)]
    Logging(#[from] PrePostError),
    #[error("invalid aws_bedrock_runtime_endpoint")]
    Endpoint {
        #[source]
        source: serde_json::Error,
    },
    #[error("the async invoke status reply is not JSON")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("Failed to get async invoke status: {status} - {}", body.0)]
    HttpFailure { status: u16, body: FailureBody },
}
