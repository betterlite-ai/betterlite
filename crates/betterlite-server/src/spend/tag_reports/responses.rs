use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use betterlite_error::envelope::{ErrorObject, ErrorResponse, WireField};
use serde::Serialize;

#[derive(Serialize)]
struct DetailMessage<'message> {
    error: &'message str,
}

#[derive(Serialize)]
struct DetailBody<'message> {
    detail: DetailMessage<'message>,
}

// the ProxyException boundary of the three tag routes
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
    (
        status,
        Json(DetailBody {
            detail: DetailMessage { error: message },
        }),
    )
        .into_response()
}

// global_view_spend_tags raises its own ProxyException with param None, before the catch-all wraps anything
pub(super) fn missing_dates() -> Response {
    let body = ErrorResponse {
        error: ErrorObject {
            message: "Please provide start_date and end_date".to_owned(),
            error_type: WireField::Text("bad_request".to_owned()),
            param: WireField::Null,
            code: WireField::Text("400".to_owned()),
        },
    };
    (StatusCode::BAD_REQUEST, Json(body)).into_response()
}
