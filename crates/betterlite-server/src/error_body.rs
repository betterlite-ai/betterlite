use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use serde::Serialize;
#[derive(Serialize)]
struct ErrorResponse {
    error: ErrorObject,
}
#[derive(Serialize)]
struct ErrorObject {
    message: String,
    #[serde(rename = "type")]
    error_type: String,
    param: Option<String>,
    code: String,
}
pub(crate) enum AuthenticationFailure {
    Missing,
    Invalid,
}
pub(crate) fn authentication_error(failure: &AuthenticationFailure) -> Response {
    match failure {
        AuthenticationFailure::Missing => {
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
        }
        AuthenticationFailure::Invalid => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: ErrorObject {
                    message: "the API key is not valid".to_owned(),
                    error_type: "no_db_connection".to_owned(),
                    param: None,
                    code: "400".to_owned(),
                },
            }),
        )
            .into_response(),
    }
}
pub(crate) fn request_timeout_error() -> Response {
    let error = ErrorResponse {
        error: ErrorObject {
            message: "request timed out".to_owned(),
            error_type: "invalid_request_error".to_owned(),
            param: None,
            code: "408".to_owned(),
        },
    };
    (StatusCode::REQUEST_TIMEOUT, Json(error)).into_response()
}
