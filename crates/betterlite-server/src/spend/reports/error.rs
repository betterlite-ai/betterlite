use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use serde::Serialize;

pub(super) const DB_NOT_CONNECTED_ERROR: &str = CommonProxyErrors::DbNotConnectedError.as_str();

// FastAPI renders an HTTPException with a string detail as {"detail": "..."}
#[derive(Serialize)]
pub(super) struct TextDetail<'message> {
    pub(super) detail: &'message str,
}

#[derive(Serialize)]
struct InternalMessage {
    message: &'static str,
    #[serde(rename = "type")]
    kind: &'static str,
}

#[derive(Serialize)]
struct InternalBody {
    error: InternalMessage,
}

// otel_unhandled_exception_handler renders an uncaught error as this one internal error body
pub(super) fn internal_error() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(InternalBody {
            error: InternalMessage {
                message: "Internal server error",
                kind: "internal_server_error",
            },
        }),
    )
        .into_response()
}
