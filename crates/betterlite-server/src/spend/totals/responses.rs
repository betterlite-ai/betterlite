use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use betterlite_error::envelope::{ErrorObject, ErrorResponse, WireField};
use serde::Serialize;

#[derive(Serialize)]
pub(super) struct ErrorBody {
    message: &'static str,
    #[serde(rename = "type")]
    kind: &'static str,
}

#[derive(Serialize)]
pub(super) struct InternalErrorBody {
    error: ErrorBody,
}

pub(super) const INTERNAL_ERROR: InternalErrorBody = InternalErrorBody {
    error: ErrorBody {
        message: "Internal server error",
        kind: "internal_server_error",
    },
};

#[derive(Serialize)]
struct DetailMessage {
    error: String,
}

#[derive(Serialize)]
struct DetailBody<Detail> {
    detail: Detail,
}

#[derive(Serialize)]
struct ValidationIssue {
    #[serde(rename = "type")]
    kind: &'static str,
    loc: [&'static str; 2],
    msg: &'static str,
}

pub(super) fn wrapped_error(message: String) -> Response {
    let body = ErrorResponse {
        error: ErrorObject {
            message,
            error_type: WireField::Text("internal_error".to_owned()),
            param: WireField::Text("None".to_owned()),
            code: WireField::Text("500".to_owned()),
        },
    };
    (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
}

pub(super) fn detail_error(status: StatusCode, message: &str) -> Response {
    let detail = DetailMessage {
        error: message.to_owned(),
    };
    (status, Json(DetailBody { detail })).into_response()
}

pub(super) fn invalid_limit_type() -> Response {
    let issue = ValidationIssue {
        kind: "int_parsing",
        loc: ["query", "limit"],
        msg: "Input should be a valid integer, unable to parse string as an integer",
    };
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(DetailBody { detail: [issue] }),
    )
        .into_response()
}
