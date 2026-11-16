use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use serde::Serialize;
use serde_json::error::Category;
use serde_path_to_error::Segment;
use thiserror::Error;

use crate::tracing_routes::router::unauthenticated;
use crate::workflow_runs::store::StoreError;

const INTERNAL_DETAIL: &str = "Internal server error";

#[derive(Debug, Error)]
pub enum RunsError {
    #[error("Run '{run_id}' not found")]
    NotFound { run_id: String },
    #[error("No fields to update")]
    NoFieldsToUpdate,
    #[error("the database is not connected")]
    DbNotConnected,
    #[error("the request has no authenticated caller")]
    Unauthenticated,
    #[error("the request does not match the declared types")]
    Validation(#[from] ValidationFailure),
    #[error("the workflow store failed")]
    Store(#[from] StoreError),
    #[error("a workflow value does not encode as JSON")]
    Encode(#[from] serde_json::Error),
    #[error("the next sequence number overflows")]
    SequenceOverflow,
}

#[derive(Debug, Error)]
pub enum ValidationFailure {
    #[error("the request body does not decode")]
    Body {
        #[source]
        source: Box<serde_path_to_error::Error<serde_json::Error>>,
    },
    #[error("the request body holds more than one JSON value")]
    Trailing {
        #[source]
        source: serde_json::Error,
    },
    #[error("the limit query parameter is not an integer")]
    LimitNotInteger,
    #[error("the limit query parameter is below 1")]
    LimitTooSmall,
    #[error("the limit query parameter is above 250")]
    LimitTooLarge,
    #[error("the query string does not decode")]
    Query,
}

#[derive(Serialize)]
struct DetailBody {
    detail: String,
}

#[derive(Serialize)]
struct ValidationBody {
    detail: [ValidationIssue; 1],
}

#[derive(Serialize)]
struct ValidationIssue {
    #[serde(rename = "type")]
    issue_type: &'static str,
    loc: Vec<LocPart>,
    msg: String,
}

#[derive(Serialize)]
#[serde(untagged)]
enum LocPart {
    Field(String),
    Index(usize),
}

impl IntoResponse for RunsError {
    fn into_response(self) -> Response {
        let (status, detail) = match self {
            Self::Validation(failure) => return failure.into_response(),
            Self::Unauthenticated => return unauthenticated(),
            Self::NotFound { run_id } => {
                (StatusCode::NOT_FOUND, format!("Run '{run_id}' not found"))
            }
            Self::NoFieldsToUpdate => (StatusCode::BAD_REQUEST, "No fields to update".to_owned()),
            Self::DbNotConnected => (
                StatusCode::INTERNAL_SERVER_ERROR,
                CommonProxyErrors::DbNotConnectedError.as_str().to_owned(),
            ),
            internal @ (Self::Store(_) | Self::Encode(_) | Self::SequenceOverflow) => {
                tracing::error!(error = ?internal, "workflow run request failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    INTERNAL_DETAIL.to_owned(),
                )
            }
        };
        (status, Json(DetailBody { detail })).into_response()
    }
}

impl IntoResponse for ValidationFailure {
    fn into_response(self) -> Response {
        let issue = match &self {
            Self::Body { source } => body_issue(source),
            Self::Trailing { .. } => ValidationIssue {
                issue_type: "json_invalid",
                loc: vec![LocPart::Field("body".to_owned())],
                msg: "JSON decode error".to_owned(),
            },
            Self::LimitNotInteger => limit_issue(
                "int_parsing",
                "Input should be a valid integer, unable to parse string as an integer",
            ),
            Self::LimitTooSmall => limit_issue(
                "greater_than_equal",
                "Input should be greater than or equal to 1",
            ),
            Self::LimitTooLarge => limit_issue(
                "less_than_equal",
                "Input should be less than or equal to 250",
            ),
            Self::Query => ValidationIssue {
                issue_type: "value_error",
                loc: vec![LocPart::Field("query".to_owned())],
                msg: "invalid query parameters".to_owned(),
            },
        };
        let body = ValidationBody { detail: [issue] };
        (StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response()
    }
}

fn limit_issue(issue_type: &'static str, msg: &str) -> ValidationIssue {
    ValidationIssue {
        issue_type,
        loc: vec![
            LocPart::Field("query".to_owned()),
            LocPart::Field("limit".to_owned()),
        ],
        msg: msg.to_owned(),
    }
}

fn body_issue(source: &serde_path_to_error::Error<serde_json::Error>) -> ValidationIssue {
    let message = source.inner().to_string();
    let mut loc = vec![LocPart::Field("body".to_owned())];
    for segment in source.path() {
        match segment {
            Segment::Seq { index } => loc.push(LocPart::Index(*index)),
            Segment::Map { key } => loc.push(LocPart::Field(key.clone())),
            Segment::Enum { variant } => loc.push(LocPart::Field(variant.clone())),
            Segment::Unknown => {}
        }
    }
    if let Some(field) = message
        .strip_prefix("missing field `")
        .and_then(|rest| rest.split_once('`'))
        .map(|(field, _position)| field)
    {
        loc.push(LocPart::Field(field.to_owned()));
        return ValidationIssue {
            issue_type: "missing",
            loc,
            msg: "Field required".to_owned(),
        };
    }
    let issue_type = match source.inner().classify() {
        Category::Syntax | Category::Eof => "json_invalid",
        Category::Io | Category::Data => "value_error",
    };
    ValidationIssue {
        issue_type,
        loc,
        msg: message,
    }
}
