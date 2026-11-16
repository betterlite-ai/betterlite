use betterlite_http::error::HttpError;
use thiserror::Error;

use super::VideoHeaders;

#[derive(Debug, Error)]
pub enum FalVideoError {
    #[error("FAL_AI_API_KEY is not set")]
    MissingApiKey,
    #[error("fal.ai needs a public image URL for input_reference")]
    ReferenceNotText,
    #[error("fal.ai seconds must be a numeric value")]
    SecondsNotNumeric,
    #[error("fal.ai video ids must be created through litellm with a model")]
    VideoIdWithoutModel,
    #[error("the Fal video reply at HTTP {status} is larger than {limit} bytes")]
    ReplyTooLarge {
        status: u16,
        headers: VideoHeaders,
        limit: usize,
    },
    #[error("the Fal video reply at HTTP {status} is not JSON")]
    ReplyDecode {
        status: u16,
        headers: VideoHeaders,
        #[source]
        source: serde_json::Error,
    },
    #[error("the Fal video reply at HTTP {status} is not a JSON object")]
    ReplyNotAnObject { status: u16, headers: VideoHeaders },
    #[error(transparent)]
    PathSegment(#[from] HttpError),
    #[error("the Fal video object does not serialize")]
    VideoObjectEncode(#[source] serde_json::Error),
    #[error("the Fal video object does not deserialize")]
    VideoObjectDecode(#[source] serde_json::Error),
}

// what get_error_class builds for an upstream error: the status, message and headers as the provider sent them
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("{message}")]
pub struct FalVideoException {
    pub status_code: u16,
    pub message: String,
    pub headers: VideoHeaders,
}

#[must_use]
pub fn get_error_class(
    error_message: &str,
    status_code: u16,
    headers: &VideoHeaders,
) -> FalVideoException {
    FalVideoException {
        status_code,
        message: error_message.to_owned(),
        headers: headers.clone(),
    }
}
