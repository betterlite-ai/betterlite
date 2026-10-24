#![cfg(test)]

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use axum::response::Response;
use rstest::rstest;
use secrecy::SecretString;
use serde::Deserialize;
use tower::ServiceExt as _;

use crate::routes::router;

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
    let app = router(Some(SecretString::from("master")));
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
async fn unknown_route_requires_master_key_before_returning_not_found() {
    let app = router(Some(SecretString::from("master")));
    let request = Request::builder()
        .uri("/not-served")
        .body(Body::empty())
        .unwrap();
    let response = send_request(app, request).await;

    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, response.status());
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
    let app = router(Some(SecretString::from("master")));
    let request = Request::builder()
        .uri("/not-served")
        .header(name, value)
        .body(Body::empty())
        .unwrap();
    let response = send_request(app, request).await;

    assert_eq!(StatusCode::NOT_FOUND, response.status(), "{name}");
}

#[tokio::test]
async fn wrong_key_has_litellm_error_shape_without_echoing_secret() {
    let app = router(Some(SecretString::from("master")));
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
