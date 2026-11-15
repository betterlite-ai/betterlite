use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::Response;
use betterlite_core::api::request_body::{OTLP_TRACE_METHOD, OTLP_TRACE_PATH};
use betterlite_logging::otlp_response::encode_otlp_response;

// LiteLLM's OTLP_RETRY_AFTER_SECONDS default; the environment override is not resolved yet
pub const OTLP_RETRY_AFTER: &str = "2";

#[must_use]
pub fn reason_phrase(status: StatusCode) -> &'static str {
    match status.as_u16() {
        // Python's http.client.responses uses the RFC 9110 names the http crate predates
        203 => "Non-Authoritative Information",
        413 => "Content Too Large",
        418 => "I'm a Teapot",
        422 => "Unprocessable Content",
        _ => status.canonical_reason().unwrap_or("Trace request failed"),
    }
}

#[must_use]
pub fn otlp_encoded_response(
    content_type: Option<&str>,
    status: StatusCode,
    message: Option<&str>,
) -> Response {
    let (body, media_type) = encode_otlp_response(content_type, message);
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(media_type));
    response
}

#[must_use]
pub fn otlp_retry_response(
    content_type: Option<&str>,
    status: StatusCode,
    message: &str,
) -> Response {
    let (mut parts, body) = otlp_encoded_response(content_type, status, Some(message)).into_parts();
    parts.headers.insert(
        header::RETRY_AFTER,
        HeaderValue::from_static(OTLP_RETRY_AFTER),
    );
    Response::from_parts(parts, body)
}

#[must_use]
pub fn otlp_error_response(
    method: &str,
    path: &str,
    content_type: Option<&str>,
    status: StatusCode,
    headers: &HeaderMap,
) -> Option<Response> {
    if method != OTLP_TRACE_METHOD || path != OTLP_TRACE_PATH {
        return None;
    }
    let mut response = otlp_encoded_response(content_type, status, Some(reason_phrase(status)));
    response.headers_mut().extend(
        headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone())),
    );
    Some(response)
}
