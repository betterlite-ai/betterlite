use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use betterlite_core::diagnostic_snapshots::unsupported::{DiagnosticError, UnsupportedSnapshot};

const UNSUPPORTED_RUNTIME: &str = "unsupported_runtime";

pub(crate) fn unsupported_runtime(message: &str) -> Response {
    let error = DiagnosticError {
        error_type: UNSUPPORTED_RUNTIME.to_owned(),
        code: UNSUPPORTED_RUNTIME.to_owned(),
        message: message.to_owned(),
    };
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(UnsupportedSnapshot { error }),
    )
        .into_response()
}
