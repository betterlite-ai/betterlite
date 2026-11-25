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
    #[error("start_date and end_date must be in YYYY-MM-DD format")]
    DateFormat,
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

#[derive(Serialize)]
struct InternalMessage {
    message: &'static str,
    #[serde(rename = "type")]
    kind: &'static str,
}

#[derive(Serialize)]
struct InternalBody {
    error: InternalMessage,
}

// otel_unhandled_exception_handler answers the strptime ValueError raised outside the handler's try
const UNHANDLED_ERROR: InternalBody = InternalBody {
    error: InternalMessage {
        message: "Internal server error",
        kind: "internal_server_error",
    },
};

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
            Self::MissingDates => detail_error(StatusCode::BAD_REQUEST, &self.to_string()),
            Self::DateFormat => {
                tracing::warn!(error = %self, "a global spend report date could not be read");
                (StatusCode::INTERNAL_SERVER_ERROR, Json(UNHANDLED_ERROR)).into_response()
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
