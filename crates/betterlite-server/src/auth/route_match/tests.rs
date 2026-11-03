#![cfg(test)]

use crate::routes;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::{Extension, State};
use axum::http::{Request, StatusCode};
use axum::routing::get;
use rstest::rstest;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tower::ServiceExt as _;

use super::RouteMatchAdmitted;

async fn downstream(
    State(calls): State<Arc<AtomicUsize>>,
    Extension(_admitted): Extension<RouteMatchAdmitted>,
) -> &'static str {
    calls.fetch_add(1, Ordering::Relaxed);
    "chat-reached"
}

fn production_router(calls: Arc<AtomicUsize>) -> Router {
    let chat = Router::new()
        .route("/v1/chat/completions", get(downstream))
        .route("/not-recognized", get(downstream))
        .route("/v1/a2a/{agent_id}/message/send", get(downstream))
        .with_state(calls);
    routes::router(
        None,
        Arc::new(AtomicBool::new(false)),
        &[],
        chat,
        Router::new(),
    )
}

fn request_to(path: &str) -> Request<Body> {
    Request::builder().uri(path).body(Body::empty()).unwrap()
}

#[tokio::test]
async fn stage_is_registered_by_the_production_constructor() {
    let calls = Arc::new(AtomicUsize::new(0));
    let response = production_router(Arc::clone(&calls))
        .oneshot(request_to("/v1/chat/completions"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&*body, b"chat-reached");
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn failure_stops_downstream_with_the_stage_error_body() {
    let calls = Arc::new(AtomicUsize::new(0));
    let response = production_router(Arc::clone(&calls))
        .oneshot(request_to("/not-recognized"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&*body, br#"{"detail":"Not Found"}"#);
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[rstest]
#[case::upper_hex("/v1/a2a/a%2Fb/message/send")]
#[case::lower_hex("/v1/a2a/a%2fb/message/send")]
#[tokio::test]
async fn encoded_separator_is_decoded_before_template_admission(#[case] path: &str) {
    let calls = Arc::new(AtomicUsize::new(0));
    let response = production_router(Arc::clone(&calls))
        .oneshot(request_to(path))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}
