use axum::Json;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse as _, Response};
use serde::Serialize;

use super::AdmissionStats;

#[derive(Serialize)]
struct OverloadedBody {
    error: OverloadedError,
}

#[derive(Serialize)]
struct OverloadedError {
    message: String,
    #[serde(rename = "type")]
    error_type: &'static str,
    code: &'static str,
}

pub(super) fn overloaded(stats: &AdmissionStats) -> Response {
    let body = OverloadedBody {
        error: OverloadedError {
            message: format!(
                "Worker at capacity: {} in-flight, {} queued requests. Retry later.",
                stats.admitted, stats.queued
            ),
            error_type: "overloaded_error",
            code: "503",
        },
    };
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(header::RETRY_AFTER, "1")],
        Json(body),
    )
        .into_response()
}
