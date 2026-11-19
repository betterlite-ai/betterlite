use axum::Json;
use axum::http::StatusCode;
use axum::http::header::{ALLOW, HeaderValue};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use thiserror::Error;

use super::date::quoted_value;

pub(crate) const DATABASE_MISSING_ERROR: &str = "Database not connected. Connect a database to your proxy - https://docs.litellm.ai/docs/simple_proxy#managing-auth---virtual-keys";

#[derive(Debug, Error)]
pub(crate) enum CacheActivityError {
    #[error("a required date parameter is missing")]
    MissingDates(Vec<&'static str>),
    #[error("the database is not connected")]
    DatabaseMissing,
    #[error("{parameter} is not a YYYY-MM-DD date")]
    InvalidDate {
        parameter: &'static str,
        value: String,
    },
}

#[derive(Serialize)]
struct DetailBody<Detail> {
    detail: Detail,
}

#[derive(Serialize)]
struct ErrorDetail {
    error: String,
}

#[derive(Serialize)]
struct MissingDatesBody {
    detail: Vec<MissingDateIssue>,
}

#[derive(Serialize)]
struct MissingDateIssue {
    #[serde(rename = "type")]
    kind: &'static str,
    loc: [&'static str; 2],
    msg: &'static str,
}

#[derive(Serialize)]
struct MethodNotAllowedBody {
    detail: &'static str,
}

impl IntoResponse for CacheActivityError {
    fn into_response(self) -> Response {
        match self {
            Self::MissingDates(parameters) => {
                let detail = parameters.into_iter().map(missing_date_issue).collect();
                (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(MissingDatesBody { detail }),
                )
                    .into_response()
            }
            Self::DatabaseMissing => {
                let detail = ErrorDetail {
                    error: DATABASE_MISSING_ERROR.to_owned(),
                };
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(DetailBody { detail }),
                )
                    .into_response()
            }
            Self::InvalidDate { parameter, value } => {
                let detail = ErrorDetail {
                    error: format!(
                        "{parameter} must be a YYYY-MM-DD date, got {}",
                        quoted_value(&value)
                    ),
                };
                (StatusCode::BAD_REQUEST, Json(DetailBody { detail })).into_response()
            }
        }
    }
}

fn missing_date_issue(name: &'static str) -> MissingDateIssue {
    MissingDateIssue {
        kind: "missing",
        loc: ["query", name],
        msg: "Field required",
    }
}

pub(crate) fn method_not_allowed() -> Response {
    let mut response = (
        StatusCode::METHOD_NOT_ALLOWED,
        Json(MethodNotAllowedBody {
            detail: "Method Not Allowed",
        }),
    )
        .into_response();
    response
        .headers_mut()
        .insert(ALLOW, HeaderValue::from_static("GET"));
    response
}
