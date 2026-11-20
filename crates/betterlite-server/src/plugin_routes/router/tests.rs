#![cfg(test)]

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};

use super::router;
use crate::plugin_routes::config::PluginRegistry;
use crate::plugin_routes::proxy::PluginState;
use crate::plugin_routes::test_support::{
    FakeUpstream, PLUGIN, authorized, master_key, registry, reply, send, state,
};

fn platform_registry() -> Arc<PluginRegistry> {
    registry(r#"{"plugins":[{"name":"litellm-platform-plugin","url":"http://plugin.local"}]}"#)
}

fn plugin_state(upstream: Arc<FakeUpstream>) -> PluginState {
    state(platform_registry(), upstream, None)
}

fn get_request(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

#[tokio::test]
async fn serves_the_plugin_list() {
    let upstream = FakeUpstream::answering(reply(StatusCode::OK, ""));
    let app = router(plugin_state(upstream));
    let (status, _headers, _body) =
        send(app, authorized(get_request("/api/plugins"), master_key())).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn serves_the_auth_token_route() {
    let upstream = FakeUpstream::answering(reply(StatusCode::OK, ""));
    let app = router(plugin_state(upstream));
    let (status, _headers, _body) = send(app, get_request("/api/plugins/auth-token")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn serves_the_proxy_with_a_trailing_slash() {
    let upstream = FakeUpstream::answering(reply(StatusCode::OK, ""));
    let app = router(plugin_state(Arc::clone(&upstream)));
    let request = authorized(
        get_request(&format!("/plugin-proxy/{PLUGIN}/")),
        master_key(),
    );
    let (status, _headers, _body) = send(app, request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(upstream.sent().len(), 1);
}

#[tokio::test]
async fn serves_the_proxy_with_a_path() {
    let upstream = FakeUpstream::answering(reply(StatusCode::OK, ""));
    let app = router(plugin_state(Arc::clone(&upstream)));
    let request = authorized(
        get_request(&format!("/plugin-proxy/{PLUGIN}/a/b")),
        master_key(),
    );
    let (status, _headers, _body) = send(app, request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(upstream.sent()[0].url, "http://plugin.local/a/b");
}

#[tokio::test]
async fn does_not_serve_the_proxy_without_a_trailing_slash() {
    let upstream = FakeUpstream::answering(reply(StatusCode::OK, ""));
    let app = router(plugin_state(Arc::clone(&upstream)));
    let request = authorized(
        get_request(&format!("/plugin-proxy/{PLUGIN}")),
        master_key(),
    );
    let (status, _headers, _body) = send(app, request).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(upstream.sent().is_empty());
}

async fn proxy_method_status(app: Router, method: Method) -> StatusCode {
    let request = authorized(
        Request::builder()
            .method(method)
            .uri(format!("/plugin-proxy/{PLUGIN}/path"))
            .body(Body::empty())
            .unwrap(),
        master_key(),
    );
    let (status, _headers, _body) = send(app, request).await;
    status
}

#[tokio::test]
async fn serves_every_method_on_the_proxy() {
    let upstream = FakeUpstream::answering(reply(StatusCode::OK, ""));
    let app = router(plugin_state(Arc::clone(&upstream)));
    assert_eq!(
        proxy_method_status(app.clone(), Method::GET).await,
        StatusCode::OK
    );
    assert_eq!(
        proxy_method_status(app.clone(), Method::POST).await,
        StatusCode::OK
    );
    assert_eq!(
        proxy_method_status(app.clone(), Method::PUT).await,
        StatusCode::OK
    );
    assert_eq!(
        proxy_method_status(app.clone(), Method::DELETE).await,
        StatusCode::OK
    );
    assert_eq!(
        proxy_method_status(app.clone(), Method::PATCH).await,
        StatusCode::OK
    );
    assert_eq!(
        proxy_method_status(app.clone(), Method::OPTIONS).await,
        StatusCode::OK
    );
    assert_eq!(proxy_method_status(app, Method::HEAD).await, StatusCode::OK);
    assert_eq!(upstream.sent().len(), 7);
}

#[tokio::test]
async fn answers_405_for_a_method_the_proxy_does_not_serve() {
    let upstream = FakeUpstream::answering(reply(StatusCode::OK, ""));
    let app = router(plugin_state(Arc::clone(&upstream)));
    let request = authorized(
        Request::builder()
            .method(Method::TRACE)
            .uri(format!("/plugin-proxy/{PLUGIN}/path"))
            .body(Body::empty())
            .unwrap(),
        master_key(),
    );
    let (status, _headers, _body) = send(app, request).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert!(upstream.sent().is_empty());
}
