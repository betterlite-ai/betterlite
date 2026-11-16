use betterlite_http::proxy::transport_error::TransportError;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use thiserror::Error;

use crate::api::fal::video_prepare::error::{FalVideoError, FalVideoException};

use super::unsupported::UnsupportedOperation;

#[derive(Debug, Error)]
pub enum VideoResultsError {
    #[error(transparent)]
    Reply(#[from] FalVideoError),
    #[error("the fal.ai result probe URL is not a URL")]
    ProbeUrl(#[source] url::ParseError),
    #[error("the fal.ai result probe failed")]
    ProbeTransport(#[source] TransportError),
    #[error("the fal.ai video artifact URL is not a URL")]
    ArtifactUrl(#[source] url::ParseError),
    #[error("the original {name} header name cannot be sent to fal.ai")]
    HeaderName {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the original {name} header value cannot be sent to fal.ai")]
    HeaderValue {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error(transparent)]
    Upstream(#[from] FalVideoException),
    #[error("fal.ai video result did not include a video URL")]
    MissingVideoUrl,
    #[error("fal.ai video result did not include a video URL: {detail}")]
    MissingVideoUrlDetail { detail: String },
    #[error("the fal.ai video artifact answered HTTP {status}")]
    ArtifactStatus { status: u16 },
    #[error("the fal.ai video artifact could not be downloaded")]
    ArtifactTransport(#[source] TransportError),
    #[error(transparent)]
    Unsupported(#[from] UnsupportedOperation),
}
