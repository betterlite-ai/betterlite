use betterlite_openai::search::credentials::CredentialError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FastcrwError {
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error("CRW_API_KEY is not set. Set `CRW_API_KEY` environment variable.")]
    MissingApiKey,
    #[error("the fastCRW search reply is larger than {limit} bytes")]
    ReplyTooLarge { limit: usize },
    #[error("the fastCRW search reply does not match the search response shape: {source}")]
    Reply {
        #[source]
        source: serde_path_to_error::Error<serde_json::Error>,
    },
    #[error("the fastCRW search result does not match the search result shape: {source}")]
    Result {
        #[source]
        source: serde_json::Error,
    },
    #[error("the fastCRW search result field {field} is not a string")]
    NotText {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the fastCRW search reply has bytes after the JSON document")]
    TrailingBytes(#[source] serde_json::Error),
}
