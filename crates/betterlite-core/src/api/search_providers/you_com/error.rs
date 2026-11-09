use betterlite_openai::search::credentials::CredentialError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum YouComError {
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error("country must be a string")]
    CountryNotString,
    #[error("description must be a string")]
    DescriptionNotString,
    #[error("the You.com search reply is larger than {limit} bytes")]
    ReplyTooLarge { limit: usize },
    #[error("the You.com search reply does not match the search response shape: {source}")]
    Reply {
        #[source]
        source: serde_path_to_error::Error<serde_json::Error>,
    },
    #[error("the You.com search reply has bytes after the JSON document")]
    TrailingBytes(#[source] serde_json::Error),
}
