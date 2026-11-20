use std::error::Error;

use betterlite_logging::logging_pre_post::error::PrePostError;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use thiserror::Error;

use crate::api::vector_store_http_write::error::{
    BaseLlmException, ClientError, FailureFacts, handle_error,
};

use super::config::CompactConfig;

// A failure from the provider config before or after the send, carried as the source it raised
#[derive(Debug, Error)]
#[error(transparent)]
pub struct ConfigFailure {
    pub source: Box<dyn Error + Send + Sync>,
}

impl ConfigFailure {
    #[must_use]
    pub fn new<Source>(source: Source) -> Self
    where
        Source: Error + Send + Sync + 'static,
    {
        Self {
            source: Box::new(source),
        }
    }
}

// Which handler step a provider config failure came from: only the send failures reach _handle_error
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Url,
    Validate,
    Transform,
    Normalize,
    Sign,
    Response,
}

impl Stage {
    const fn text(self) -> &'static str {
        match self {
            Self::Url => "URL resolution",
            Self::Validate => "environment validation",
            Self::Transform => "request transform",
            Self::Normalize => "request normalization",
            Self::Sign => "request signing",
            Self::Response => "response transform",
        }
    }
}

#[derive(Debug, Error)]
pub enum CompactError {
    #[error(transparent)]
    Provider(#[from] BaseLlmException),
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error(transparent)]
    Logging(#[from] PrePostError),
    #[error("{} failed: {source}", .stage.text())]
    Step { stage: Stage, source: ConfigFailure },
    #[error("the request URL {url} is not a URL")]
    RequestUrl {
        url: String,
        #[source]
        source: url::ParseError,
    },
    #[error("the {name} header name is not a header name")]
    HeaderName {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the {name} header value is not a header value")]
    HeaderValue {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the request body could not be encoded")]
    Encode(#[source] serde_json::Error),
}

// _handle_error's funnel: an HTTP error keeps its status, text and masked headers; anything else becomes a synthesized 500
#[must_use]
pub fn provider_error<C>(facts: &FailureFacts, config: &C) -> BaseLlmException
where
    C: CompactConfig,
{
    let fallback = handle_error(facts, None);
    let mut error = config.error_class(&fallback.message, fallback.status_code, fallback.headers);
    if fallback.status_code_is_synthesized {
        error.status_code_is_synthesized = true;
    }
    error
}
