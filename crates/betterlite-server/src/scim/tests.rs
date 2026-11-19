#![cfg(test)]

use crate::routes;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use rstest::rstest;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tower::ServiceExt as _;

#[rstest]
#[case::bulk("/scim/v2/Bulk")]
#[case::me("/scim/v2/Me")]
#[tokio::test]
async fn unserved_scim_path_answers_not_found(#[case] path: &str) {
    let config = routes::tests::empty_config();
    let application = routes::router(
        routes::tests::state_for(&config),
        Arc::new(AtomicBool::new(false)),
        Router::new(),
        Router::new(),
    );
    let response = application
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&*body, br#"{"detail":"Not Found"}"#);
}
