use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use thiserror::Error;

use crate::roi_github::error::SourceError;
use crate::roi_settings::error::RoiSettingsError;
use crate::tracing_routes::router::unauthenticated;

#[derive(Debug, Error)]
pub enum RoiConnectionsError {
    #[error("the request has no authenticated caller")]
    Unauthenticated,
    #[error("{0}")]
    Forbidden(&'static str),
    #[error("{0}")]
    Conflict(&'static str),
    #[error("{0}")]
    BadGateway(#[from] SourceError),
    #[error("the request does not match the declared types")]
    Query(Vec<QueryIssue>),
    #[error(transparent)]
    Settings(#[from] RoiSettingsError),
}

#[derive(Debug, Serialize)]
pub struct QueryIssue {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub loc: Vec<&'static str>,
    pub msg: String,
}

#[derive(Serialize)]
struct DetailBody {
    detail: String,
}

#[derive(Serialize)]
struct IssuesBody {
    detail: Vec<QueryIssue>,
}

impl IntoResponse for RoiConnectionsError {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthenticated => unauthenticated(),
            Self::Forbidden(detail) => detail_response(StatusCode::FORBIDDEN, detail),
            Self::Conflict(detail) => detail_response(StatusCode::CONFLICT, detail),
            Self::BadGateway(source) => {
                detail_response(StatusCode::BAD_GATEWAY, source.to_string().as_str())
            }
            Self::Query(issues) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(IssuesBody { detail: issues }),
            )
                .into_response(),
            Self::Settings(error) => error.into_response(),
        }
    }
}

fn detail_response(status: StatusCode, detail: &str) -> Response {
    let body = DetailBody {
        detail: detail.to_owned(),
    };
    (status, Json(body)).into_response()
}
