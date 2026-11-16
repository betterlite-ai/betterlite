use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use betterlite_error::envelope::{ErrorObject, ErrorResponse, WireField};
use serde::Serialize;

#[derive(Serialize)]
struct ErrorBody {
    message: &'static str,
    #[serde(rename = "type")]
    kind: &'static str,
}

#[derive(Serialize)]
struct InternalErrorBody {
    error: ErrorBody,
}

#[derive(Serialize)]
struct DetailMessage<'a> {
    error: &'a str,
}

#[derive(Serialize)]
struct DetailBody<'a> {
    detail: DetailMessage<'a>,
}

#[derive(Serialize)]
struct ValidationIssue {
    #[serde(rename = "type")]
    kind: &'static str,
    loc: [&'static str; 2],
    msg: &'static str,
}

#[derive(Serialize)]
struct ValidationBody {
    detail: [ValidationIssue; 1],
}

pub(super) fn unauthenticated() -> Response {
    let body = ErrorResponse {
        error: ErrorObject {
            message: "the API key is not valid".to_owned(),
            error_type: WireField::Text("auth_error".to_owned()),
            param: WireField::MISSING,
            code: WireField::Text("401".to_owned()),
        },
    };
    (StatusCode::UNAUTHORIZED, Json(body)).into_response()
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

pub(super) fn detail_error(message: &str) -> Response {
    let body = DetailBody {
        detail: DetailMessage { error: message },
    };
    (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
}

pub(super) fn internal_error() -> Response {
    let body = InternalErrorBody {
        error: ErrorBody {
            message: "Internal server error",
            kind: "internal_server_error",
        },
    };
    (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
}

pub(super) fn invalid_limit(limit: i64) -> Response {
    let message = format!("Invalid limit: {limit}, error: Limit must be greater than 0");
    let body = DetailBody {
        detail: DetailMessage { error: &message },
    };
    (StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response()
}

pub(super) fn invalid_limit_type() -> Response {
    let body = ValidationBody {
        detail: [ValidationIssue {
            kind: "int_parsing",
            loc: ["query", "limit"],
            msg: "Input should be a valid integer, unable to parse string as an integer",
        }],
    };
    (StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response()
}
