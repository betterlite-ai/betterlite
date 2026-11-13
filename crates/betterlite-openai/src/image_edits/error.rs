use http::header::{InvalidHeaderName, InvalidHeaderValue};
use thiserror::Error;

use crate::compatible_wire::error::OpenAiLikeError;
use crate::image_generation::reply::ImageReplyError;

pub const BAD_REQUEST_STATUS: u16 = 400;

#[derive(Debug, Error)]
pub enum ImageEditError {
    #[error("the api_base is not a valid URL")]
    ApiBase(#[source] url::ParseError),
    #[error("a supplied header name is not valid")]
    HeaderName(#[source] InvalidHeaderName),
    #[error("a supplied header value cannot carry its text")]
    HeaderValue(#[source] InvalidHeaderValue),
    #[error("the Authorization header cannot carry the resolved key")]
    Authorization(#[source] InvalidHeaderValue),
    #[error("DALL-E-2 only supports editing a single image. Please provide one image.")]
    MultipleImages { model: String },
    #[error(transparent)]
    OpenAi(#[from] OpenAiLikeError),
    #[error(transparent)]
    Reply(#[from] ImageReplyError),
}

impl ImageEditError {
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::MultipleImages { .. } => Some(BAD_REQUEST_STATUS),
            Self::OpenAi(error) => Some(error.status_code),
            Self::Reply(ImageReplyError::ResponseObject { status, .. }) => Some(*status),
            Self::ApiBase(_)
            | Self::HeaderName(_)
            | Self::HeaderValue(_)
            | Self::Authorization(_)
            | Self::Reply(_) => None,
        }
    }
}
