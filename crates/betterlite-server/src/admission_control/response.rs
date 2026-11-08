use axum::body::Body;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
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
    let mut response = Response::new(Body::from(serde_json::to_vec(&body).unwrap_or_default()));
    *response.status_mut() = StatusCode::SERVICE_UNAVAILABLE;
    response
        .headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response
}
