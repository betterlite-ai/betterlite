use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EstimateError {
    #[error("the request body does not match the declared types")]
    Request(Vec<Issue>),
    #[error("{message}")]
    Calculation { message: String },
}

// LiteLLM's validation handler sends only type, loc and msg; the calculation detail carries its own error text.
#[derive(Debug, Serialize)]
pub struct Issue {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub loc: Vec<String>,
    pub msg: String,
}

#[derive(Debug, Serialize)]
struct DetailBody {
    detail: Detail,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum Detail {
    Issues(Vec<Issue>),
    Error(DetailError),
}

#[derive(Debug, Serialize)]
struct DetailError {
    error: String,
}

impl IntoResponse for EstimateError {
    fn into_response(self) -> Response {
        match self {
            Self::Request(issues) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(DetailBody {
                    detail: Detail::Issues(issues),
                }),
            )
                .into_response(),
            Self::Calculation { message } => (
                StatusCode::NOT_FOUND,
                Json(DetailBody {
                    detail: Detail::Error(DetailError { error: message }),
                }),
            )
                .into_response(),
        }
    }
}
