use betterlite_http::error::HttpError;
use thiserror::Error;

use crate::error::ProviderError;

#[derive(Debug, Error)]
pub enum EvalsError {
    #[error("OPENAI_API_KEY is required for Evals API")]
    ApiKeyRequired,
    #[error("{source}")]
    PathSegment {
        #[source]
        source: HttpError,
    },
    #[error("the eval reply is {size} bytes, over the {limit} byte limit")]
    ReplyTooLarge { size: usize, limit: usize },
    #[error("the eval reply is not valid JSON: {source}")]
    ReplyJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("the eval reply does not match its model: {source}")]
    ReplyModel {
        #[source]
        source: serde_json::Error,
    },
    #[error("the eval request does not serialize: {source}")]
    RequestEncode {
        #[source]
        source: serde_json::Error,
    },
    #[error("a typed eval request is not an object")]
    RequestNotObject,
    #[error("{}", .provider.message)]
    Provider { provider: Box<ProviderError> },
}

impl From<ProviderError> for EvalsError {
    fn from(provider: ProviderError) -> Self {
        Self::Provider {
            provider: Box::new(provider),
        }
    }
}
