use axum::body::to_bytes;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use rstest::rstest;

use crate::tracing_routes::otlp::{
    OTLP_RETRY_AFTER, otlp_encoded_response, otlp_error_response, otlp_retry_response,
    reason_phrase,
};

#[rstest]
#[case::bad_request(400, "Bad Request")]
#[case::non_authoritative(203, "Non-Authoritative Information")]
#[case::content_too_large(413, "Content Too Large")]
#[case::teapot(418, "I'm a Teapot")]
#[case::unprocessable(422, "Unprocessable Content")]
#[case::internal(500, "Internal Server Error")]
#[case::unknown(599, "Trace request failed")]
fn reason_phrases_match_python(#[case] code: u16, #[case] expected: &str) {
    let status = StatusCode::from_u16(code).expect("status");
    assert_eq!(reason_phrase(status), expected);
}

#[test]
fn a_non_trace_request_gets_no_otlp_response() {
    let headers = HeaderMap::new();
    assert!(
        otlp_error_response("GET", "/v1/traces", None, StatusCode::NOT_FOUND, &headers).is_none()
    );
    assert!(
        otlp_error_response(
            "POST",
            "/v1/traces/x",
            None,
            StatusCode::NOT_FOUND,
            &headers
        )
        .is_none()
    );
}

#[tokio::test]
async fn a_json_error_carries_the_reason_phrase_and_the_caller_headers() {
    let mut headers = HeaderMap::new();
    headers.insert("x-request-id", HeaderValue::from_static("req-1"));
    let response = otlp_error_response(
        "POST",
        "/v1/traces",
        Some("application/json"),
        StatusCode::UNPROCESSABLE_ENTITY,
        &headers,
    )
    .expect("otlp response");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("application/json"))
    );
    assert_eq!(
        response.headers().get("x-request-id"),
        Some(&HeaderValue::from_static("req-1"))
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    assert_eq!(body.as_ref(), b"{\"message\": \"Unprocessable Content\"}");
}

#[tokio::test]
async fn a_protobuf_error_uses_the_protobuf_media_type() {
    let response = otlp_error_response(
        "POST",
        "/v1/traces",
        None,
        StatusCode::TOO_MANY_REQUESTS,
        &HeaderMap::new(),
    )
    .expect("otlp response");
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("application/x-protobuf"))
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    assert!(!body.is_empty());
}

#[tokio::test]
async fn a_successful_json_response_is_an_empty_object() {
    let response = otlp_encoded_response(
        Some("application/json; charset=utf-8"),
        StatusCode::OK,
        None,
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    assert_eq!(body.as_ref(), b"{}");
}

#[test]
fn a_retry_response_sets_retry_after() {
    let response = otlp_retry_response(
        Some("application/json"),
        StatusCode::SERVICE_UNAVAILABLE,
        "Trace ingestion is temporarily unavailable",
    );
    assert_eq!(
        response.headers().get(header::RETRY_AFTER),
        Some(&HeaderValue::from_static(OTLP_RETRY_AFTER))
    );
}
