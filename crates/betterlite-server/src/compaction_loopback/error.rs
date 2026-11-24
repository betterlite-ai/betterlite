use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use thiserror::Error;

use crate::error_body::json_error;

mod tests;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LoopbackError {
    #[error("Native compaction child request failed (HTTP {status})")]
    ChildFailed { status: u16 },
    #[error("Native compaction child returned an invalid JSON object")]
    InvalidChildBody,
    #[error("the native compaction payload has no string model name")]
    MissingModel,
}

impl IntoResponse for LoopbackError {
    fn into_response(self) -> Response {
        json_error(
            StatusCode::BAD_REQUEST,
            &self.to_string(),
            "invalid_request_error",
            "400",
        )
    }
}
