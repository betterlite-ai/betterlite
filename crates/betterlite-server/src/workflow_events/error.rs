use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use thiserror::Error;

use crate::error_body::json_error;
use crate::workflow_runs::error::{RunsError, ValidationFailure};
use crate::workflow_runs::store::StoreError;

pub const CONFLICT_DETAIL: &str = "Concurrent write conflict \u{2014} please retry";

#[derive(Debug, Error)]
pub enum EventsError {
    #[error(transparent)]
    Runs(#[from] RunsError),
    #[error("a concurrent write kept taking the sequence number this append asked for")]
    Conflict,
    #[error("the limit query parameter is out of range")]
    Limit(#[from] LimitIssue),
    #[error("the workflow store failed during the run lookup")]
    RunLookup(#[source] StoreError),
}

impl EventsError {
    // LiteLLM calls _require_run outside its try block, so a store failure there reaches the unhandled-exception handler
    pub(super) fn run_lookup(error: RunsError) -> Self {
        match error {
            RunsError::Store(source) => Self::RunLookup(source),
            other @ (RunsError::NotFound { .. }
            | RunsError::NoFieldsToUpdate
            | RunsError::DbNotConnected
            | RunsError::Unauthenticated
            | RunsError::Validation(_)
            | RunsError::Encode(_)
            | RunsError::SequenceOverflow) => Self::Runs(other),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum LimitIssue {
    #[error("the limit query parameter is not an integer")]
    NotInteger,
    #[error("the limit query parameter is below 1")]
    TooSmall,
    #[error("the limit query parameter is above 500")]
    TooLarge,
}

#[derive(Serialize)]
struct DetailBody {
    detail: String,
}

#[derive(Serialize)]
struct LimitBody {
    detail: [LimitIssueBody; 1],
}

#[derive(Serialize)]
struct LimitIssueBody {
    #[serde(rename = "type")]
    kind: &'static str,
    loc: [&'static str; 2],
    msg: &'static str,
}

impl LimitIssue {
    const fn body(self) -> LimitIssueBody {
        match self {
            Self::NotInteger => LimitIssueBody {
                kind: "int_parsing",
                loc: ["query", "limit"],
                msg: "Input should be a valid integer, unable to parse string as an integer",
            },
            Self::TooSmall => LimitIssueBody {
                kind: "greater_than_equal",
                loc: ["query", "limit"],
                msg: "Input should be greater than or equal to 1",
            },
            Self::TooLarge => LimitIssueBody {
                kind: "less_than_equal",
                loc: ["query", "limit"],
                msg: "Input should be less than or equal to 500",
            },
        }
    }
}

impl From<ValidationFailure> for EventsError {
    fn from(source: ValidationFailure) -> Self {
        Self::Runs(RunsError::Validation(source))
    }
}

impl From<StoreError> for EventsError {
    fn from(source: StoreError) -> Self {
        Self::Runs(RunsError::Store(source))
    }
}

impl From<serde_json::Error> for EventsError {
    fn from(source: serde_json::Error) -> Self {
        Self::Runs(RunsError::Encode(source))
    }
}

impl IntoResponse for EventsError {
    fn into_response(self) -> Response {
        match self {
            Self::Runs(error) => error.into_response(),
            Self::RunLookup(source) => {
                tracing::error!(error = ?source, "workflow event run lookup failed");
                json_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error",
                    "internal_server_error",
                    "500",
                )
            }
            Self::Conflict => (
                StatusCode::CONFLICT,
                Json(DetailBody {
                    detail: CONFLICT_DETAIL.to_owned(),
                }),
            )
                .into_response(),
            Self::Limit(issue) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(LimitBody {
                    detail: [issue.body()],
                }),
            )
                .into_response(),
        }
    }
}
