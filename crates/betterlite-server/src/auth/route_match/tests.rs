#![cfg(test)]

use crate::auth::auth_route_patterns::AuthSettings;
use crate::routes;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::{Extension, State};
use axum::http::{Request, StatusCode};
use axum::routing::get;
use betterlite_config::sections::auth_route_patterns::Settings;
use rstest::rstest;
use std::fs;
use std::path::PathBuf;
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
        AuthSettings {
            master_key: None,
            routes: Settings::default(),
        },
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

#[rstest]
#[case::slash("/a%2Fb", Some("/a/b"))]
#[case::newline("/a%0Ab", Some("/a\nb"))]
#[case::unicode("/%C3%A9", Some("/\u{e9}"))]
#[case::invalid_utf8("/%FF", None)]
#[case::malformed_escape("/%GG%", Some("/%GG%"))]
#[case::single_decode("/%252F", Some("/%2F"))]
fn path_decoder(#[case] encoded: &str, #[case] expected: Option<&str>) {
    assert_eq!(super::decoded_route_path(encoded).as_deref(), expected);
}

#[test]
fn path_decoder_size_limit() {
    assert!(super::decoded_route_path(&"/".repeat(8_192)).is_some());
    assert!(super::decoded_route_path(&"/".repeat(8_193)).is_none());
}

#[rstest]
fn replays_the_route_path_corpus(#[files("fuzz/corpus/route_path/*")] path: PathBuf) {
    let encoded = fs::read_to_string(path).unwrap();
    if let Some(decoded) = super::decoded_route_path(&encoded) {
        assert!(encoded.len() <= 8_192);
        assert!(decoded.len() <= encoded.len());
    }
}
