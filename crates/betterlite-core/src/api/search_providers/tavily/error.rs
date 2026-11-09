use betterlite_openai::search::credentials::CredentialError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TavilyError {
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error("TAVILY_API_KEY is not set. Set `TAVILY_API_KEY` environment variable.")]
    MissingApiKey,
    #[error("country must be a string")]
    CountryNotString,
    #[error("the Tavily search reply is larger than {limit} bytes")]
    ReplyTooLarge { limit: usize },
    #[error("the Tavily search reply does not match the search response shape: {source}")]
    Reply {
        #[source]
        source: serde_path_to_error::Error<serde_json::Error>,
    },
    #[error("the Tavily search reply has bytes after the JSON document")]
    TrailingBytes(#[source] serde_json::Error),
}
