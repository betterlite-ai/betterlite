#![cfg(test)]

use axum::Router;
use axum::http::StatusCode;
use rstest::rstest;

use crate::debug_routes::test_support;

fn router_with(profile: Option<&str>) -> Router {
    let environment =
        profile.map_or_else(test_support::empty_environment, test_support::environment);
    test_support::protected(test_support::router(test_support::state(), &environment))
}

#[rstest]
#[case::unset(None)]
#[case::empty(Some(""))]
#[case::false_value(Some("false"))]
#[case::padded(Some(" true"))]
#[tokio::test]
async fn is_absent_without_a_profiling_flag(#[case] profile: Option<&str>) {
    let (status, text) = test_support::call(
        router_with(profile),
        "GET",
        "/memory-usage",
        Some(test_support::MASTER_KEY),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(!text.contains("unsupported_runtime"));
}

#[rstest]
#[case::true_value(Some("true"))]
#[case::upper(Some("TRUE"))]
#[tokio::test]
async fn answers_501_when_profiling_is_on(#[case] profile: Option<&str>) {
    let (status, text) = test_support::call(
        router_with(profile),
        "GET",
        "/memory-usage",
        Some(test_support::MASTER_KEY),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    assert!(text.contains("\"type\":\"unsupported_runtime\""));
}

#[tokio::test]
async fn refuses_a_request_without_a_key() {
    let router = test_support::router(
        test_support::state(),
        &test_support::environment("true"),
    );
    let (status, text) = test_support::call(router, "GET", "/memory-usage", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(text.contains("\"type\":\"auth_error\""));
}
