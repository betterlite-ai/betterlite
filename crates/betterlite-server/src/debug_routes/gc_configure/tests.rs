#![cfg(test)]

use axum::Router;
use axum::http::{StatusCode, Uri};

use crate::debug_routes::test_support;

#[test]
fn defaults_are_seven_hundred_ten_and_ten() {
    let uri = "/debug/memory/gc/configure".parse::<Uri>().expect("a URI");
    assert_eq!(
        super::parse(&uri).expect("defaults parse"),
        [700_i64, 10, 10]
    );
}

#[test]
fn reads_explicit_generations() {
    let uri = "/debug/memory/gc/configure?generation_0=500&generation_1=20&generation_2=5"
        .parse::<Uri>()
        .expect("a URI");
    assert_eq!(
        super::parse(&uri).expect("explicit values parse"),
        [500_i64, 20, 5]
    );
}

fn protected_router() -> Router {
    test_support::protected(test_support::router(test_support::state(), None))
}

#[tokio::test]
async fn answers_501_behind_the_key_middleware() {
    let (status, text) = test_support::call(
        protected_router(),
        "POST",
        "/debug/memory/gc/configure",
        Some(test_support::MASTER_KEY),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    assert!(text.contains("\"type\":\"unsupported_runtime\""));
    assert!(text.contains("\"code\":\"unsupported_runtime\""));
}

#[tokio::test]
async fn rejects_a_non_integer_generation() {
    let (status, text) = test_support::call(
        protected_router(),
        "POST",
        "/debug/memory/gc/configure?generation_1=abc",
        Some(test_support::MASTER_KEY),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(text.contains("\"type\":\"int_parsing\""));
    assert!(text.contains("\"loc\":[\"query\",\"generation_1\"]"));
}

#[tokio::test]
async fn refuses_a_request_without_a_key() {
    let router = test_support::router(test_support::state(), None);
    let (status, text) =
        test_support::call(router, "POST", "/debug/memory/gc/configure", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(text.contains("\"type\":\"auth_error\""));
    assert!(!text.contains("unsupported_runtime"));
}
