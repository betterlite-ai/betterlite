use betterlite_openai::cloudflare_chat::error::CloudflareChatError;
use betterlite_openai::decisions::transformation::requests::RequestConversionError;
use betterlite_openai::perplexity_embedding::error::PerplexityEmbeddingError;
use betterlite_openai::perplexity_responses::error::PerplexityResponsesError;
use betterlite_openai::perplexity_search::error::PerplexitySearchError;
use thiserror::Error;

use super::handler::HandlerError;

#[derive(Debug, Error)]
pub enum DispatchError {
    #[error(transparent)]
    Handler(#[from] HandlerError),
    #[error(transparent)]
    Cloudflare(#[from] CloudflareChatError),
    #[error("the call could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    PerplexityEmbedding(Box<PerplexityEmbeddingError>),
    #[error(transparent)]
    PerplexityResponses(Box<PerplexityResponsesError>),
    #[error(transparent)]
    PerplexitySearch(Box<PerplexitySearchError>),
    #[error(transparent)]
    Decisions(Box<RequestConversionError>),
}
