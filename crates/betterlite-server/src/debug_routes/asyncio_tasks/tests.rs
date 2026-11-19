#![cfg(test)]

use axum::http::StatusCode;

use crate::debug_routes::test_support;

#[tokio::test]
async fn refuses_a_request_without_a_key() {
    let router = test_support::router(test_support::state(), None);
    let (status, text) = test_support::call(router, "GET", "/debug/asyncio-tasks", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(text.contains("\"type\":\"auth_error\""));
}

#[tokio::test]
async fn answers_501_behind_the_key_middleware() {
    let router = test_support::protected(test_support::router(test_support::state(), None));
    let (status, text) = test_support::call(
        router,
        "GET",
        "/debug/asyncio-tasks",
        Some(test_support::MASTER_KEY),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    assert!(text.contains("\"type\":\"unsupported_runtime\""));
    assert!(text.contains("\"code\":\"unsupported_runtime\""));
}
