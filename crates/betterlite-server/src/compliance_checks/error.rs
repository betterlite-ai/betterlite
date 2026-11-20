use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_auth::resolved_key::auth::AuthError;
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ComplianceError {
    #[error("the request body does not match the declared types")]
    Request(Vec<Issue>),
    #[error("the management default auth could not be built")]
    DefaultAuth(#[source] AuthError),
}

// LiteLLM's validation handler sends only type, loc and msg; a list index is the one numeric loc part.
#[derive(Debug, Eq, PartialEq, Serialize)]
pub struct Issue {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub loc: Vec<LocPart>,
    pub msg: String,
}

#[derive(Debug, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum LocPart {
    Name(&'static str),
    Index(usize),
}

#[derive(Debug, Serialize)]
struct DetailBody {
    detail: Vec<Issue>,
}

impl IntoResponse for ComplianceError {
    fn into_response(self) -> Response {
        match self {
            Self::Request(issues) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(DetailBody { detail: issues }),
            )
                .into_response(),
            Self::DefaultAuth(_error) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
            }
        }
    }
}
