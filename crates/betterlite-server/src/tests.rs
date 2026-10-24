#![cfg(test)]
use crate::routes::router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use axum::response::Response;
use rstest::rstest;
use secrecy::SecretString;
use serde::Deserialize;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tower::ServiceExt as _;
#[derive(Deserialize)]
struct ErrorResponse {
    error: ErrorObject,
}
#[derive(Deserialize)]
struct ErrorObject {
    #[serde(rename = "type")]
    error_type: String,
    param: Option<String>,
    code: String,
}
async fn send_request(app: axum::Router, request: Request<Body>) -> Response {
    app.oneshot(request).await.unwrap()
}
#[tokio::test]
async fn health_routes_are_public_and_return_liveliness_body() {
    let app = router(
        Some(SecretString::from("master")),
        Arc::new(AtomicBool::new(false)),
    );
    let request = Request::builder()
        .uri("/health/liveliness")
        .body(Body::empty())
        .unwrap();
    let response = send_request(app, request).await;
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    assert_eq!(StatusCode::OK, status);
    assert_eq!(body.as_ref(), b"\"I'm alive!\"");
}
#[tokio::test]
async fn health_routes_report_shutdown_state() {
    let shutting_down = Arc::new(AtomicBool::new(true));
    let app = router(Some(SecretString::from("master")), shutting_down);
    let request = Request::builder()
        .uri("/health/liveliness")
        .body(Body::empty())
        .unwrap();
    let response = send_request(app, request).await;
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    assert_eq!(StatusCode::SERVICE_UNAVAILABLE, status);
    assert_eq!(body.as_ref(), br#"{"status":"shutting_down"}"#);
}
#[tokio::test]
async fn unknown_route_requires_master_key_before_returning_not_found() {
    let app = router(
        Some(SecretString::from("master")),
        Arc::new(AtomicBool::new(false)),
    );
    let request = Request::builder()
        .uri("/not-served")
        .body(Body::empty())
        .unwrap();
    let response = send_request(app, request).await;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, response.status());
}
#[tokio::test]
async fn missing_master_key_passes_requests_to_routing() {
    let app = router(None, Arc::new(AtomicBool::new(false)));
    let request = Request::builder()
        .uri("/not-served")
        .body(Body::empty())
        .unwrap();
    let response = send_request(app, request).await;
    assert_eq!(StatusCode::NOT_FOUND, response.status());
}
#[rstest]
#[case::authorization("authorization", "Bearer master")]
#[case::api_key("api-key", "master")]
#[case::x_api_key("x-api-key", "master")]
#[case::x_goog_api_key("x-goog-api-key", "master")]
#[case::ocp_apim("ocp-apim-subscription-key", "master")]
#[case::x_litellm_api_key("x-litellm-api-key", "master")]
#[tokio::test]
async fn every_supported_header_authenticates_unknown_route(
    #[case] name: &str,
    #[case] value: &str,
) {
    let app = router(
        Some(SecretString::from("master")),
        Arc::new(AtomicBool::new(false)),
    );
    let request = Request::builder()
        .uri("/not-served")
        .header(name, value)
        .body(Body::empty())
        .unwrap();
    let response = send_request(app, request).await;
    assert_eq!(StatusCode::NOT_FOUND, response.status(), "{name}");
}
#[tokio::test]
async fn litellm_header_precedes_authorization() {
    let app = router(
        Some(SecretString::from("master")),
        Arc::new(AtomicBool::new(false)),
    );
    let request = Request::builder()
        .uri("/not-served")
        .header("authorization", "Bearer wrong")
        .header("x-litellm-api-key", "Bearer master")
        .body(Body::empty())
        .unwrap();
    let response = send_request(app, request).await;
    assert_eq!(StatusCode::NOT_FOUND, response.status());
}
#[rstest]
#[case::lowercase_bearer("bearer master")]
#[case::basic("Basic master")]
#[case::aws("AWS4-HMAC-SHA256 Credential=Bearer master/20261024/eu/service/aws4_request")]
#[tokio::test]
async fn authorization_prefixes_match_litellm(#[case] value: &str) {
    let app = router(
        Some(SecretString::from("master")),
        Arc::new(AtomicBool::new(false)),
    );
    let request = Request::builder()
        .uri("/not-served")
        .header("authorization", value)
        .body(Body::empty())
        .unwrap();
    let response = send_request(app, request).await;
    assert_eq!(StatusCode::NOT_FOUND, response.status(), "{value}");
}
#[tokio::test]
async fn empty_authorization_falls_through_to_other_headers() {
    let app = router(
        Some(SecretString::from("master")),
        Arc::new(AtomicBool::new(false)),
    );
    let request = Request::builder()
        .uri("/not-served")
        .header("authorization", "")
        .header("x-api-key", "master")
        .body(Body::empty())
        .unwrap();
    let response = send_request(app, request).await;
    assert_eq!(StatusCode::NOT_FOUND, response.status());
}
#[tokio::test]
async fn authorization_without_prefix_is_malformed() {
    let app = router(
        Some(SecretString::from("master")),
        Arc::new(AtomicBool::new(false)),
    );
    let request = Request::builder()
        .uri("/not-served")
        .header("authorization", "master")
        .body(Body::empty())
        .unwrap();
    let response = send_request(app, request).await;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, response.status());
}
#[tokio::test]
async fn wrong_key_has_litellm_error_shape_without_echoing_secret() {
    let app = router(
        Some(SecretString::from("master")),
        Arc::new(AtomicBool::new(false)),
    );
    let request = Request::builder()
        .uri("/not-served")
        .header("x-api-key", "wrong")
        .body(Body::empty())
        .unwrap();
    let response = send_request(app, request).await;
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    let json: ErrorResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(StatusCode::BAD_REQUEST, status);
    assert_eq!(json.error.error_type, "no_db_connection");
    assert_eq!(json.error.param, None);
    assert_eq!(json.error.code, "400");
    assert!(!body.windows(5).any(|window| window == b"wrong"));
}
