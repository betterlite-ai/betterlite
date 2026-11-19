#![cfg(test)]

use axum::Router;
use axum::http::{StatusCode, Uri};

use crate::debug_routes::tests::test_support;

#[test]
fn defaults_are_twenty_and_process_info() {
    let uri = "/debug/memory/details".parse::<Uri>().expect("a URI");
    assert_eq!(super::parse(&uri).expect("defaults parse"), (20_i64, true));
}

#[test]
fn reads_explicit_values() {
    let uri = "/debug/memory/details?top_n=3&include_process_info=false"
        .parse::<Uri>()
        .expect("a URI");
    assert_eq!(
        super::parse(&uri).expect("explicit values parse"),
        (3_i64, false)
    );
}

#[test]
fn reads_an_explicit_true_process_flag() {
    let uri = "/debug/memory/details?include_process_info=yes"
        .parse::<Uri>()
        .expect("a URI");
    assert_eq!(
        super::parse(&uri).expect("an explicit true parses"),
        (20_i64, true)
    );
}

fn protected_router() -> Router {
    test_support::protected(test_support::router(test_support::state(), None))
}

#[tokio::test]
async fn answers_501_behind_the_key_middleware() {
    let (status, text) = test_support::call(
        protected_router(),
        "GET",
        "/debug/memory/details",
        Some(test_support::MASTER_KEY),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    assert!(text.contains("\"type\":\"unsupported_runtime\""));
    assert!(text.contains("\"code\":\"unsupported_runtime\""));
}

#[tokio::test]
async fn rejects_a_non_integer_top_n() {
    let (status, text) = test_support::call(
        protected_router(),
        "GET",
        "/debug/memory/details?top_n=abc",
        Some(test_support::MASTER_KEY),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(text.contains("\"type\":\"int_parsing\""));
    assert!(text.contains("\"loc\":[\"query\",\"top_n\"]"));
}

#[tokio::test]
async fn rejects_a_non_boolean_process_flag() {
    let (status, text) = test_support::call(
        protected_router(),
        "GET",
        "/debug/memory/details?include_process_info=maybe",
        Some(test_support::MASTER_KEY),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(text.contains("\"type\":\"bool_parsing\""));
    assert!(text.contains("\"loc\":[\"query\",\"include_process_info\"]"));
}

#[tokio::test]
async fn refuses_a_request_without_a_key_before_reading_the_query() {
    let router = test_support::router(test_support::state(), None);
    let (status, text) =
        test_support::call(router, "GET", "/debug/memory/details?top_n=abc", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(text.contains("\"type\":\"auth_error\""));
    assert!(!text.contains("int_parsing"));
}
