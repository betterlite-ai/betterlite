use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use thiserror::Error;

use crate::vertex_veo_operation::error::VeoOperationError;

#[derive(Debug, Error)]
pub enum StudioVeoOperationError {
    #[error("the Veo operation response is {size} bytes, over the {limit} byte limit")]
    ResponseTooLarge { size: usize, limit: usize },
    #[error("the Veo operation response is not valid JSON")]
    Json(#[source] serde_json::Error),
    #[error("No operation name in Veo response")]
    MissingName,
    #[error(
        "Video generation is not complete yet. Please check status with video_status() before downloading."
    )]
    NotComplete,
    #[error("No response data in completed operation")]
    NoResponse,
    #[error("the completed operation has no generated samples")]
    NoSamples,
    #[error("the operation URL {url} is not valid")]
    Url {
        url: String,
        #[source]
        source: url::ParseError,
    },
    #[error("the provider could not be reached at {url}")]
    Transport {
        url: String,
        #[source]
        source: TransportError,
    },
    #[error("the provider answered {} with status {}", .url, .response.status)]
    Provider {
        url: String,
        response: Box<UpstreamResponse>,
    },
    #[error(transparent)]
    Usage(#[from] VeoOperationError),
    #[error("{message}")]
    Unsupported { message: &'static str },
}
