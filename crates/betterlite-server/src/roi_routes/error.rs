use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_db::postgres_store::errors::DbError;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use betterlite_roi::summary::error::SummaryError;
use serde::Serialize;
use thiserror::Error;

use crate::roi_connections::error::{QueryIssue, RoiConnectionsError};
use crate::roi_settings::error::RoiSettingsError;

pub const NOT_READY: &str = "Connect GitHub, select repositories, and choose a router model.";
pub const ALREADY_RUNNING: &str = "A sync is already running.";
pub const RESTART_BLOCKED: &str = "Cancel the running analysis before restarting setup.";
pub const INVALID_IDENTITY: &str = "Enter a GitHub login and a valid email address.";
pub const FIELD_REQUIRED: &str = "Field required";
pub const JSON_DECODE_ERROR: &str = "JSON decode error";
pub const MODEL_ATTRIBUTES_MESSAGE: &str =
    "Input should be a valid dictionary or object to extract fields from";
pub const MODE_LITERAL: &str = "Input should be 'live' or 'demo'";
pub const UNHANDLED_MESSAGE: &str = "Internal server error";

#[derive(Debug, Error)]
pub enum RoiRoutesError {
    #[error(transparent)]
    Access(#[from] RoiConnectionsError),
    #[error(transparent)]
    Settings(#[from] RoiSettingsError),
    #[error("{0}")]
    Conflict(&'static str),
    #[error("the request does not match the declared types")]
    Request(Vec<QueryIssue>),
    #[error("Enter a GitHub login and a valid email address.")]
    InvalidIdentity,
    #[error("the stored sync time {value:?} is not a date-time")]
    Timestamp { value: String },
    #[error("the next update time is outside the supported range")]
    TimestampOverflow,
    #[error("the sync store failed")]
    Store(#[from] DbError<serde_json::Error>),
    #[error("the report summary failed")]
    Summary(#[from] SummaryError),
    #[error("the database is not connected")]
    DbNotConnected,
}

pub fn issue(kind: &'static str, loc: Vec<&'static str>, msg: &str) -> QueryIssue {
    QueryIssue {
        kind,
        loc,
        msg: msg.to_owned(),
    }
}

#[derive(Serialize)]
struct DetailBody {
    detail: String,
}

#[derive(Serialize)]
struct IssuesBody {
    detail: Vec<QueryIssue>,
}

#[derive(Serialize)]
struct InternalBody {
    error: InternalMessage,
}

#[derive(Serialize)]
struct InternalMessage {
    message: &'static str,
    #[serde(rename = "type")]
    error_type: &'static str,
}

impl IntoResponse for RoiRoutesError {
    fn into_response(self) -> Response {
        match self {
            Self::Access(error) => error.into_response(),
            Self::Settings(error) => error.into_response(),
            Self::Conflict(detail) => detail_response(StatusCode::CONFLICT, detail),
            Self::Request(issues) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(IssuesBody { detail: issues }),
            )
                .into_response(),
            Self::InvalidIdentity => {
                detail_response(StatusCode::UNPROCESSABLE_ENTITY, INVALID_IDENTITY)
            }
            Self::DbNotConnected => detail_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                CommonProxyErrors::DbNotConnectedError.as_str(),
            ),
            Self::Timestamp { value } => unhandled(&format!(
                "the stored sync time {value:?} is not a date-time"
            )),
            Self::TimestampOverflow => {
                unhandled("the next update time is outside the supported range")
            }
            Self::Store(error) => unhandled(&error.to_string()),
            Self::Summary(error) => unhandled(&error.to_string()),
        }
    }
}

fn detail_response(status: StatusCode, detail: &str) -> Response {
    (
        status,
        Json(DetailBody {
            detail: detail.to_owned(),
        }),
    )
        .into_response()
}

fn unhandled(message: &str) -> Response {
    tracing::error!(error = message, "the ROI calculator request failed");
    let body = InternalBody {
        error: InternalMessage {
            message: UNHANDLED_MESSAGE,
            error_type: "internal_server_error",
        },
    };
    (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
}
