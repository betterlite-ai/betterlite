use axum::Json;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::Response;
use serde::Serialize;
use serde_json::error::Category;
use serde_path_to_error::Segment;
use thiserror::Error;

use crate::endpoints::passthrough_config_reads::record::EndpointRecordError;
use crate::passthrough::route_registration::RouteRegistrationError;
use crate::tracing_routes::router::unauthenticated;

use super::settings_port::GeneralSettingsError;

const NO_ENDPOINTS: &str = "No pass-through endpoints found";
const NO_SAVED_ENDPOINTS: &str = "There are no pass-through endpoints setup.";
const FIELD_REQUIRED: &str = "Field required";
const JSON_DECODE_ERROR: &str = "JSON decode error";
const INTERNAL_MESSAGE: &str = "Internal server error";
const INTERNAL_TYPE: &str = "internal_server_error";

// the update route answers 404 where the delete route answers 400, and the texts differ too
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LookupKind {
    Update,
    Delete,
}

#[derive(Debug, Error)]
pub enum PassthroughWriteError {
    #[error("the request has no authenticated caller")]
    Unauthenticated,
    #[error("the stored pass-through endpoints are not a list")]
    StoredNotAList,
    #[error("a stored pass-through endpoint cannot be decoded")]
    Record(#[source] EndpointRecordError),
    #[error("no pass-through endpoints are saved")]
    NoEndpoints { kind: LookupKind },
    #[error("pass-through endpoint '{endpoint_id}' was not found")]
    MissingEndpoint {
        endpoint_id: String,
        kind: LookupKind,
    },
    #[error("the pass-through endpoint index for '{endpoint_id}' was not found")]
    MissingIndex {
        endpoint_id: String,
        kind: LookupKind,
    },
    #[error("the request body is empty")]
    MissingBody,
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
    #[error("the request has no endpoint_id query parameter")]
    MissingEndpointId,
    #[error("the request query string does not decode")]
    InvalidQuery,
    #[error("the general settings store refused the call")]
    Settings(#[source] GeneralSettingsError),
    #[error("the pass-through route cannot be registered")]
    Registration(#[source] RouteRegistrationError),
    #[error("a pass-through endpoint value cannot be encoded")]
    Encode(#[source] serde_json::Error),
}

impl IntoResponse for PassthroughWriteError {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthenticated => unauthenticated(),
            Self::Settings(error) => detail_response(error.status, error.detail),
            Self::NoEndpoints { kind } => no_endpoints_response(kind),
            Self::MissingEndpoint { endpoint_id, kind } => {
                missing_endpoint_response(kind, &endpoint_id)
            }
            Self::MissingIndex { endpoint_id, kind } => missing_index_response(kind, &endpoint_id),
            Self::MissingBody => issue_response(missing_issue(&["body"])),
            Self::MissingEndpointId => issue_response(missing_issue(&["query", "endpoint_id"])),
            Self::Trailing { .. } => issue_response(json_invalid_issue()),
            Self::InvalidQuery => issue_response(invalid_query_issue()),
            Self::Body { source } => issue_response(body_issue(&source)),
            internal @ (Self::StoredNotAList
            | Self::Record(_)
            | Self::Encode(_)
            | Self::Registration(_)) => internal_response(&internal),
        }
    }
}

fn no_endpoints_response(kind: LookupKind) -> Response {
    match kind {
        LookupKind::Update => detail_response(StatusCode::NOT_FOUND, NO_ENDPOINTS.to_owned()),
        LookupKind::Delete => {
            detail_response(StatusCode::BAD_REQUEST, NO_SAVED_ENDPOINTS.to_owned())
        }
    }
}

fn missing_endpoint_response(kind: LookupKind, endpoint_id: &str) -> Response {
    match kind {
        LookupKind::Update => detail_response(
            StatusCode::NOT_FOUND,
            format!("Endpoint with ID '{endpoint_id}' not found"),
        ),
        LookupKind::Delete => detail_response(
            StatusCode::BAD_REQUEST,
            format!(
                "Endpoint with ID '{endpoint_id}' was not found in pass-through endpoint list."
            ),
        ),
    }
}

fn missing_index_response(kind: LookupKind, endpoint_id: &str) -> Response {
    match kind {
        LookupKind::Update => detail_response(
            StatusCode::NOT_FOUND,
            format!("Could not find index for endpoint with ID '{endpoint_id}'"),
        ),
        LookupKind::Delete => detail_response(
            StatusCode::BAD_REQUEST,
            format!("Could not find index for endpoint with ID '{endpoint_id}'"),
        ),
    }
}

fn detail_response(status: StatusCode, detail: String) -> Response {
    let body = DetailBody {
        detail: DetailError { error: detail },
    };
    (status, Json(body)).into_response()
}

fn internal_response(error: &PassthroughWriteError) -> Response {
    tracing::error!(error = ?error, "the pass-through endpoint write failed");
    let body = InternalBody {
        error: InternalMessage {
            message: INTERNAL_MESSAGE,
            error_type: INTERNAL_TYPE,
        },
    };
    (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
}

fn issue_response(issue: Issue) -> Response {
    let body = IssuesBody {
        detail: vec![issue],
    };
    (StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response()
}

fn missing_issue(parts: &[&str]) -> Issue {
    let loc = parts
        .iter()
        .map(|part| LocPart::Field((*part).to_owned()))
        .collect();
    Issue {
        kind: "missing",
        loc,
        msg: FIELD_REQUIRED.to_owned(),
    }
}

fn json_invalid_issue() -> Issue {
    Issue {
        kind: "json_invalid",
        loc: vec![LocPart::Field("body".to_owned())],
        msg: JSON_DECODE_ERROR.to_owned(),
    }
}

fn invalid_query_issue() -> Issue {
    Issue {
        kind: "value_error",
        loc: vec![LocPart::Field("query".to_owned())],
        msg: "invalid query parameters".to_owned(),
    }
}

fn body_issue(source: &serde_path_to_error::Error<serde_json::Error>) -> Issue {
    if matches!(source.inner().classify(), Category::Syntax | Category::Eof) {
        return json_invalid_issue();
    }
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
    let missing = message
        .strip_prefix("missing field `")
        .and_then(|rest| rest.split_once('`'))
        .map(|(field, _position)| field);
    if let Some(field) = missing {
        loc.push(LocPart::Field(field.to_owned()));
        return Issue {
            kind: "missing",
            loc,
            msg: FIELD_REQUIRED.to_owned(),
        };
    }
    Issue {
        kind: "value_error",
        loc,
        msg: message,
    }
}

#[derive(Serialize)]
struct DetailBody {
    detail: DetailError,
}

#[derive(Serialize)]
struct DetailError {
    error: String,
}

#[derive(Serialize)]
struct IssuesBody {
    detail: Vec<Issue>,
}

#[derive(Serialize)]
struct Issue {
    #[serde(rename = "type")]
    kind: &'static str,
    loc: Vec<LocPart>,
    msg: String,
}

#[derive(Serialize)]
#[serde(untagged)]
enum LocPart {
    Field(String),
    Index(usize),
}

// otel_unhandled_exception_handler's body for an uncaught exception: no param or code fields
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
