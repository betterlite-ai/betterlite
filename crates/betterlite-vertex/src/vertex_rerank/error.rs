use betterlite_logging::analytics::python::FormatError;
use betterlite_models::price_fields::decimal_lexeme::DecimalLexemeError;
use thiserror::Error;

use crate::token_cache::error::CredentialCacheError;

#[derive(Debug, Error)]
pub enum VertexRerankError {
    #[error("{field} is required for Vertex AI rerank")]
    MissingField { field: &'static str },
    #[error("the {0} value must be {1}")]
    FieldType(&'static str, &'static str),
    #[error("the document at index {index} must be text or an object")]
    DocumentType { index: usize },
    #[error("the {field} of the document at index {index} must be text")]
    DocumentField { index: usize, field: &'static str },
    #[error("the document at index {index} cannot be written as text")]
    DocumentText { index: usize, source: FormatError },
    #[error("the vertex credential lookup failed")]
    Credentials(#[source] CredentialCacheError),
    #[error(
        "Vertex AI project ID is required. Please set 'VERTEXAI_PROJECT', 'litellm.vertex_project', or pass 'vertex_project' parameter"
    )]
    MissingProject,
    #[error("the rerank response is {size} bytes, over the limit")]
    TooLarge { size: usize },
    #[error("Failed to parse response: {0}")]
    Json(#[source] serde_json::Error),
    #[error("the rerank response must be a JSON object")]
    NotObject,
    #[error("the records of the response must be a list")]
    RecordsNotList,
    #[error("the record at position {position} must be an object")]
    RecordNotObject { position: usize },
    #[error("the record at position {position} has no integer id")]
    RecordId { position: usize },
    #[error("the score of the record at position {position} must be a number")]
    ScoreType { position: usize },
    #[error("the score of the record at position {position} is out of range")]
    Score {
        position: usize,
        source: DecimalLexemeError,
    },
}
