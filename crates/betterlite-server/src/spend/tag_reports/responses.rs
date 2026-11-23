use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use betterlite_error::envelope::{ErrorObject, ErrorResponse, WireField};

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
