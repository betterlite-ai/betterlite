use betterlite_config::sections::vertex_credential_sources::credential_source::CredentialDecodeError;
use betterlite_http::error::HttpError;
use thiserror::Error;

use crate::api::vertex_location::VertexLocationError;

use super::ports::PortError;

#[derive(Debug, Error)]
pub enum VertexIngestionError {
    #[error(
        "vertex_project is required for Vertex AI RAG ingestion. Set it in vector_store config."
    )]
    MissingProject,
    #[error(transparent)]
    Location(#[from] VertexLocationError),
    #[error(transparent)]
    CorpusId(#[from] HttpError),
    #[error("the vertex credentials could not be read")]
    Credentials {
        #[source]
        source: CredentialDecodeError,
    },
    #[error(transparent)]
    Port(#[from] PortError),
    #[error("the {step} response is not valid JSON")]
    Decode {
        step: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the {step} response is not a JSON object")]
    Shape { step: &'static str },
    #[error("the {step} payload cannot be written as JSON")]
    Write {
        step: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("a provider JSON value cannot be converted")]
    Value {
        #[source]
        source: serde_json::Error,
    },
    #[error("`vector_db_config` must be an object")]
    VectorDbConfig,
    #[error("`{field}` must be a string")]
    TextField { field: &'static str },
    #[error("`{field}` must be a boolean")]
    BooleanField { field: &'static str },
    #[error("`{field}` must be an integer")]
    IntegerField { field: &'static str },
    #[error("Failed to create RAG corpus: {body}")]
    CreateCorpus { body: String },
    #[error("Failed to poll operation: {body}")]
    Poll { body: String },
    #[error("Operation failed: {error}")]
    OperationFailed { error: String },
    #[error("No corpus name in operation response: {operation}")]
    MissingCorpusName { operation: String },
    #[error("Operation timed out after 30 attempts")]
    OperationTimeout,
    #[error("Failed to upload file: {body}")]
    Upload { body: String },
}
