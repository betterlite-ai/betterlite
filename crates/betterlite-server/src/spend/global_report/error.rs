use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use serde::Serialize;
use thiserror::Error;

use crate::spend::reports::store::ScopedReportError;
use crate::spend::totals::responses::detail_error;

use super::query::InvalidGroupBy;

#[derive(Debug, Error)]
pub(super) enum GlobalReportError {
    #[error(transparent)]
    GroupBy(#[from] InvalidGroupBy),
    #[error("Please provide start_date and end_date")]
    MissingDates,
    #[error("start_date must be in YYYY-MM-DD format")]
    StartDateFormat,
    #[error("end_date must be in YYYY-MM-DD format")]
    EndDateFormat,
    #[error("the database is not connected")]
    DbNotConnected,
    #[error(transparent)]
    Query(#[from] ScopedReportError),
}

// public_validation_errors keeps only the type, loc and msg of each pydantic error
#[derive(Serialize)]
struct ValidationIssue {
    #[serde(rename = "type")]
    kind: &'static str,
    loc: [&'static str; 2],
    msg: String,
}

#[derive(Serialize)]
struct ValidationBody {
    detail: [ValidationIssue; 1],
}

impl IntoResponse for GlobalReportError {
    fn into_response(self) -> Response {
        match self {
            Self::GroupBy(error) => {
                let issue = ValidationIssue {
                    kind: "literal_error",
                    loc: ["query", "group_by"],
                    msg: error.to_string(),
                };
                let body = ValidationBody { detail: [issue] };
                (StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response()
            }
            Self::MissingDates | Self::StartDateFormat | Self::EndDateFormat => {
                detail_error(StatusCode::BAD_REQUEST, &self.to_string())
            }
            Self::DbNotConnected => detail_error(
                StatusCode::BAD_REQUEST,
                CommonProxyErrors::DbNotConnectedError.as_str(),
            ),
            Self::Query(error) => {
                tracing::warn!(error = %error, "a global spend report query failed");
                detail_error(StatusCode::BAD_REQUEST, &error.to_string())
            }
        }
    }
}
