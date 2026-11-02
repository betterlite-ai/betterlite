#![cfg(test)]

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use axum::routing::get;
use tower::ServiceExt as _;

use super::stage;

async fn chat_marker() -> &'static str {
    "chat-reached"
}

fn guarded_chat() -> Router {
    Router::new()
        .route("/v1/chat/completions", get(chat_marker))
        .layer(axum::middleware::from_fn(stage))
}

fn request_to(path: &str) -> Request<Body> {
    Request::builder().uri(path).body(Body::empty()).unwrap()
}

#[tokio::test]
async fn stage_is_registered() {
    let response = guarded_chat()
        .oneshot(request_to("/v1/chat/completions"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&*body, b"chat-reached");
}

#[tokio::test]
async fn failure_stops_downstream() {
    let response = guarded_chat()
        .oneshot(request_to("/totally/not/a/registered/route"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(
        !body
            .as_ref()
            .windows(12)
            .any(|window| window == b"chat-reached")
    );
}
