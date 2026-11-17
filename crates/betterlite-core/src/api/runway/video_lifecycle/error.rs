use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use thiserror::Error;

use crate::api::runway::tasks::error::RunwayTaskError;
use crate::api::runway::video_create::error::RunwayMLError;

#[derive(Debug, Error)]
pub enum VideoLifecycleError {
    #[error(transparent)]
    Task(#[from] RunwayTaskError),
    #[error(transparent)]
    TaskId(#[from] HttpError),
    #[error("the selected artifact URL is not valid")]
    ArtifactUrl(#[source] url::ParseError),
    #[error(transparent)]
    Artifact(#[from] TransportError),
    #[error(transparent)]
    ArtifactStatus(#[from] RunwayMLError),
    #[error("{message}")]
    Unsupported { message: &'static str },
    #[error("Video is still processing (status: {status}). Please wait and try again.")]
    StillProcessing { status: String },
    #[error("Video generation failed: {reason}")]
    GenerationFailed { reason: String },
    #[error("the failure value must be a string")]
    FailureNotText,
    #[error("the output value must be a string or a list of strings")]
    OutputNotText,
    #[error("Video URL not found in response. Video may not be ready yet.")]
    MissingUrl,
}
