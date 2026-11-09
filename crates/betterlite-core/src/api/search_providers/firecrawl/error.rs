use betterlite_openai::search::credentials::CredentialError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FirecrawlError {
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error("FIRECRAWL_API_KEY is not set. Set `FIRECRAWL_API_KEY` environment variable.")]
    MissingApiKey,
    #[error("the Firecrawl search reply is larger than {limit} bytes")]
    ReplyTooLarge { limit: usize },
    #[error("the Firecrawl search reply does not match the search response shape: {source}")]
    Reply {
        #[source]
        source: serde_path_to_error::Error<serde_json::Error>,
    },
    #[error("the Firecrawl search result does not match the search result shape: {source}")]
    Result {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Firecrawl search result field {field} is not a string")]
    NotText {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the Firecrawl search reply has bytes after the JSON document")]
    TrailingBytes(#[source] serde_json::Error),
}
