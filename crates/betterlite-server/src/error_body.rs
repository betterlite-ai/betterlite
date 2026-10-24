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
    Malformed,
    Invalid,
}
pub(crate) fn authentication_error(failure: &AuthenticationFailure) -> Response {
    match failure {
        AuthenticationFailure::Missing | AuthenticationFailure::Malformed => {
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
        }
        AuthenticationFailure::Invalid => error_response(
            StatusCode::BAD_REQUEST,
            "the API key is not valid".to_owned(),
            "no_db_connection",
            "400",
        ),
    }
}
pub(crate) async fn not_found_error() -> Response {
    let body = NotFoundBody {
        detail: "Not Found",
    };
    (StatusCode::NOT_FOUND, Json(body)).into_response()
}
#[derive(Serialize)]
struct NotFoundBody {
    detail: &'static str,
}
fn error_response(status: StatusCode, message: String, error_type: &str, code: &str) -> Response {
    let body = ErrorResponse {
        error: ErrorObject {
            message,
            error_type: error_type.to_owned(),
            param: None,
            code: code.to_owned(),
        },
    };
    (status, Json(body)).into_response()
}
