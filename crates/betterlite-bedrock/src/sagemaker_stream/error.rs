use std::str::Utf8Error;

use betterlite_http::error::HttpError;
use thiserror::Error;

use crate::event_frames::error::EventFrameError;
#[derive(Debug, Error)]
pub enum SagemakerStreamError {
    #[error("the SageMaker event frame could not be read")]
    Frame(#[source] EventFrameError),
    #[error("Bad response code, expected 200: {payload}")]
    Status { payload: String },
    #[error("the SageMaker stream event is not valid UTF-8")]
    Utf8(#[source] Utf8Error),
    #[error("the SageMaker stream chunk does not have the chat chunk shape")]
    Json(#[source] serde_json::Error),
    #[error("the SageMaker stream chunk is not a JSON object")]
    NotObject,
    #[error("the SageMaker stream chunk field {field} is invalid")]
    Field { field: &'static str },
    #[error("the SageMaker response body could not be read")]
    Body(#[source] HttpError),
}
