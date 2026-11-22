use thiserror::Error;

use crate::search::credentials::CredentialError;

#[derive(Debug, Error)]
pub enum PerplexitySearchError {
    #[error("PERPLEXITYAI_API_KEY is not set. Set `PERPLEXITYAI_API_KEY` environment variable.")]
    MissingApiKey,
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error("the Perplexity search option `{field}` has the wrong type: {source}")]
    OptionType {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the Perplexity search reply is larger than {limit} bytes")]
    ReplyTooLarge { limit: usize },
    #[error("the Perplexity search reply does not match the search schema: {source}")]
    Reply {
        #[source]
        source: serde_json::Error,
    },
}
