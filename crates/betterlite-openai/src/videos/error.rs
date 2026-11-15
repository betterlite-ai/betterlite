use thiserror::Error;

use crate::error::{ProviderError, ProviderErrorParts};
use crate::openai_base_config::Headers;

#[derive(Debug, Error)]
pub enum VideoError {
    #[error("video_id is required")]
    MissingVideoId,
    #[error("video_id cannot be a dot path segment")]
    DotSegment,
    #[error("a video reply does not parse: {source}")]
    Reply {
        #[source]
        source: serde_json::Error,
    },
    #[error("the reply is not a video object")]
    ExpectedVideo,
}

impl From<serde_json::Error> for VideoError {
    fn from(source: serde_json::Error) -> Self {
        Self::Reply { source }
    }
}

// get_error_class: every field comes from the caller, and B2202 supplies the synthetic request
#[must_use]
pub fn get_error_class(message: &str, status_code: u16, headers: Headers) -> ProviderError {
    ProviderError::from(ProviderErrorParts {
        status_code,
        message: message.to_owned(),
        headers: Some(headers.into_iter().collect()),
        request: None,
        response: None,
        body: None,
        status_code_is_synthesized: false,
    })
}
