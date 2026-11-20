#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, Method, Request, StatusCode};

use super::{PluginReply, PluginState, REQUEST_BODY_LIMIT, target_url};
use crate::plugin_routes::config::PluginRegistry;
use crate::plugin_routes::headers::KeyHeaderName;
use crate::plugin_routes::router::router;
use crate::plugin_routes::test_support::{
    FakeUpstream, KeyHeaderSource, authorized, caller, key_caller, master_key, registry, reply,
    send, state, text,
};

fn platform_registry() -> Arc<PluginRegistry> {
    registry(
        r#"{"plugins":[{"name":"litellm-platform-plugin","display_name":"Platform","url":"http://plugin.local/","plugin_key":"sk-plugin"}]}"#,
    )
}

fn plugin_state(upstream: Arc<FakeUpstream>) -> PluginState {
    state(platform_registry(), upstream, None)
}

fn get_request(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

fn method_request(method: Method, uri: &str, body: Body) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .body(body)
        .unwrap()
}

#[tokio::test]
async fn forwards_the_method_path_and_query() {
    let upstream = FakeUpstream::answering(reply(StatusCode::OK, "ok"));
    let app = router(plugin_state(Arc::clone(&upstream)));
    let request = authorized(
        method_request(
            Method::POST,
            "/plugin-proxy/litellm-platform-plugin/orders/42?mode=fast",
            Body::from("hello"),
        ),
        master_key(),
    );
    let (status, _headers, body) = send(app, request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(text(&body), "ok");
    let sent = upstream.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, Method::POST);
    assert_eq!(sent[0].url, "http://plugin.local/orders/42?mode=fast");
    assert_eq!(sent[0].body, b"hello".to_vec());
}

#[tokio::test]
async fn forwards_an_empty_path_on_the_trailing_slash_route() {
    let upstream = FakeUpstream::answering(reply(StatusCode::OK, "ok"));
    let app = router(plugin_state(Arc::clone(&upstream)));
    let request = authorized(
        get_request("/plugin-proxy/litellm-platform-plugin/"),
        master_key(),
    );
    let (_status, _headers, _body) = send(app, request).await;
    let sent = upstream.sent();
    assert_eq!(sent[0].url, "http://plugin.local/");
}

async fn forward_method(app: Router, method: Method) -> StatusCode {
    let request = authorized(
        method_request(
            method,
            "/plugin-proxy/litellm-platform-plugin/path",
            Body::empty(),
        ),
        master_key(),
    );
    let (status, _headers, _body) = send(app, request).await;
    status
}

#[tokio::test]
async fn forwards_every_supported_method() {
    let upstream = FakeUpstream::answering(reply(StatusCode::OK, ""));
    let app = router(plugin_state(Arc::clone(&upstream)));
    for method in [
        Method::GET,
        Method::POST,
        Method::PUT,
        Method::DELETE,
        Method::PATCH,
        Method::OPTIONS,
    ] {
        assert_eq!(forward_method(app.clone(), method).await, StatusCode::OK);
    }
    assert_eq!(forward_method(app, Method::HEAD).await, StatusCode::OK);
    let sent = upstream.sent();
    let forwarded: Vec<Method> = sent.iter().map(|request| request.method.clone()).collect();
    assert_eq!(
        forwarded,
        [
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::PATCH,
            Method::OPTIONS,
            Method::HEAD,
        ]
    );
}

#[tokio::test]
async fn forwards_the_plugin_key_and_the_caller_identity() {
    let upstream = FakeUpstream::answering(reply(StatusCode::OK, ""));
    let app = router(plugin_state(Arc::clone(&upstream)));
    let mut request = authorized(
        get_request("/plugin-proxy/litellm-platform-plugin/"),
        master_key(),
    );
    request.headers_mut().insert(
        "authorization",
        HeaderValue::from_static("Bearer sk-caller"),
    );
    request
        .headers_mut()
        .insert("x-litellm-api-key", HeaderValue::from_static("sk-caller"));
    request
        .headers_mut()
        .insert("x-keep", HeaderValue::from_static("yes"));
    let (_status, _headers, _body) = send(app, request).await;
    let sent = upstream.sent();
    let headers = &sent[0].headers;
    assert_eq!(headers["authorization"], "Bearer sk-plugin");
    assert_eq!(headers["x-litellm-user-id"], "default_user_id");
    assert_eq!(headers["x-litellm-user-role"], "proxy_admin");
    assert_eq!(headers["x-keep"], "yes");
    assert!(!headers.contains_key("x-litellm-api-key"));
}

#[tokio::test]
async fn answers_403_before_looking_up_the_plugin() {
    let upstream = FakeUpstream::answering(reply(StatusCode::OK, ""));
    let app = router(plugin_state(Arc::clone(&upstream)));
    let request = authorized(
        get_request("/plugin-proxy/missing/path"),
        key_caller(r#"{"user_id":"u9","user_role":"internal_user"}"#),
    );
    let (status, headers, body) = send(app, request).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        text(&body),
        "Plugin proxy access requires proxy_admin role."
    );
    assert!(!headers.contains_key("content-type"));
    assert!(upstream.sent().is_empty());
}

#[tokio::test]
async fn answers_403_for_a_viewer() {
    let upstream = FakeUpstream::answering(reply(StatusCode::OK, ""));
    let app = router(plugin_state(Arc::clone(&upstream)));
    let request = authorized(
        get_request("/plugin-proxy/litellm-platform-plugin/"),
        key_caller(r#"{"user_id":"u9","user_role":"proxy_admin_viewer"}"#),
    );
    let (status, _headers, body) = send(app, request).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        text(&body),
        "Plugin proxy access requires proxy_admin role."
    );
}

#[tokio::test]
async fn answers_403_when_the_caller_has_no_role() {
    let upstream = FakeUpstream::answering(reply(StatusCode::OK, ""));
    let app = router(plugin_state(Arc::clone(&upstream)));
    let request = authorized(
        get_request("/plugin-proxy/litellm-platform-plugin/"),
        caller("unroled", Vec::new()),
    );
    let (status, _headers, _body) = send(app, request).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn answers_404_for_an_unregistered_plugin() {
    let upstream = FakeUpstream::answering(reply(StatusCode::OK, ""));
    let app = router(plugin_state(Arc::clone(&upstream)));
    let request = authorized(get_request("/plugin-proxy/missing/path"), master_key());
    let (status, _headers, body) = send(app, request).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(text(&body), "Plugin 'missing' not registered");
    assert!(upstream.sent().is_empty());
}

#[tokio::test]
async fn answers_502_when_the_upstream_fails() {
    let upstream = FakeUpstream::failing();
    let app = router(plugin_state(Arc::clone(&upstream)));
    let request = authorized(
        get_request("/plugin-proxy/litellm-platform-plugin/path"),
        master_key(),
    );
    let (status, _headers, body) = send(app, request).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(
        text(&body),
        "Cannot connect to plugin 'litellm-platform-plugin' at http://plugin.local/"
    );
}

#[tokio::test]
async fn answers_502_when_the_plugin_url_is_invalid() {
    let plugins = registry(r#"{"plugins":[{"name":"litellm-platform-plugin","url":"not a url"}]}"#);
    let app = router(PluginState::production(plugins, &BTreeMap::new(), None));
    let request = authorized(
        get_request("/plugin-proxy/litellm-platform-plugin/path"),
        master_key(),
    );
    let (status, _headers, body) = send(app, request).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(
        text(&body),
        "Cannot connect to plugin 'litellm-platform-plugin' at not a url"
    );
}

#[tokio::test]
async fn answers_502_when_a_forwarded_header_is_invalid() {
    let upstream = FakeUpstream::answering(reply(StatusCode::OK, ""));
    let plugins = registry(
        r#"{"plugins":[{"name":"litellm-platform-plugin","url":"http://plugin.local","plugin_key":"sk\nplugin"}]}"#,
    );
    let concrete: Arc<FakeUpstream> = Arc::clone(&upstream);
    let app = router(state(plugins, concrete, None));
    let request = authorized(
        get_request("/plugin-proxy/litellm-platform-plugin/path"),
        master_key(),
    );
    let (status, _headers, body) = send(app, request).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(
        text(&body),
        "Cannot connect to plugin 'litellm-platform-plugin' at http://plugin.local"
    );
    assert!(upstream.sent().is_empty());
}

#[tokio::test]
async fn answers_413_for_a_body_over_the_limit() {
    let upstream = FakeUpstream::answering(reply(StatusCode::OK, ""));
    let app = router(plugin_state(Arc::clone(&upstream)));
    let payload = Body::from(vec![0_u8; REQUEST_BODY_LIMIT + 1]);
    let request = authorized(
        method_request(
            Method::POST,
            "/plugin-proxy/litellm-platform-plugin/path",
            payload,
        ),
        master_key(),
    );
    let (status, _headers, body) = send(app, request).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(text(&body), "Request body too large");
    assert!(upstream.sent().is_empty());
}

#[tokio::test]
async fn passes_a_redirect_response_through() {
    let mut answer_headers = HeaderMap::new();
    answer_headers.insert(
        "location",
        HeaderValue::from_static("http://elsewhere.local"),
    );
    let answer = PluginReply {
        status: StatusCode::FOUND,
        headers: answer_headers,
        body: Vec::new(),
    };
    let upstream = FakeUpstream::answering(answer);
    let app = router(plugin_state(Arc::clone(&upstream)));
    let request = authorized(
        get_request("/plugin-proxy/litellm-platform-plugin/path"),
        master_key(),
    );
    let (status, headers, _body) = send(app, request).await;
    assert_eq!(status, StatusCode::FOUND);
    assert_eq!(headers["location"], "http://elsewhere.local");
    assert_eq!(upstream.sent().len(), 1);
}

#[tokio::test]
async fn drops_unsafe_response_headers_and_forces_the_sandbox() {
    let mut answer_headers = HeaderMap::new();
    answer_headers.insert("content-encoding", HeaderValue::from_static("gzip"));
    answer_headers.insert("transfer-encoding", HeaderValue::from_static("chunked"));
    answer_headers.insert("content-length", HeaderValue::from_static("10"));
    answer_headers.insert("set-cookie", HeaderValue::from_static("session=abc"));
    answer_headers.insert(
        "content-security-policy",
        HeaderValue::from_static("default-src *"),
    );
    let answer = PluginReply {
        status: StatusCode::OK,
        headers: answer_headers,
        body: b"page".to_vec(),
    };
    let upstream = FakeUpstream::answering(answer);
    let app = router(plugin_state(Arc::clone(&upstream)));
    let request = authorized(
        get_request("/plugin-proxy/litellm-platform-plugin/path"),
        master_key(),
    );
    let (status, headers, body) = send(app, request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(text(&body), "page");
    assert_eq!(headers["content-security-policy"], "sandbox");
    assert_eq!(headers["x-content-type-options"], "nosniff");
    assert!(!headers.contains_key("content-encoding"));
    assert!(!headers.contains_key("transfer-encoding"));
    assert!(!headers.contains_key("set-cookie"));
}

#[tokio::test]
async fn reads_the_custom_key_header_on_every_request() {
    let upstream = FakeUpstream::answering(reply(StatusCode::OK, ""));
    let source = KeyHeaderSource::named("X-Custom-Key");
    let upstream_clone: Arc<FakeUpstream> = Arc::clone(&upstream);
    let source_clone: Arc<KeyHeaderSource> = Arc::clone(&source);
    let header: Arc<dyn KeyHeaderName> = source_clone;
    let app = router(state(platform_registry(), upstream_clone, Some(header)));
    let mut first = authorized(
        get_request("/plugin-proxy/litellm-platform-plugin/path"),
        master_key(),
    );
    first
        .headers_mut()
        .insert("x-custom-key", HeaderValue::from_static("sk-caller"));
    let (_first_status, _first_headers, _first_body) = send(app.clone(), first).await;
    source.rename(None);
    let mut second = authorized(
        get_request("/plugin-proxy/litellm-platform-plugin/path"),
        master_key(),
    );
    second
        .headers_mut()
        .insert("x-custom-key", HeaderValue::from_static("sk-caller"));
    let (_second_status, _second_headers, _second_body) = send(app, second).await;
    let sent = upstream.sent();
    assert!(!sent[0].headers.contains_key("x-custom-key"));
    assert_eq!(sent[1].headers["x-custom-key"], "sk-caller");
}

#[tokio::test]
async fn rejects_an_unauthenticated_request() {
    let upstream = FakeUpstream::answering(reply(StatusCode::OK, ""));
    let app = router(plugin_state(Arc::clone(&upstream)));
    let (status, _headers, _body) = send(
        app,
        get_request("/plugin-proxy/litellm-platform-plugin/path"),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(upstream.sent().is_empty());
}

#[test]
fn builds_the_target_url() {
    assert_eq!(
        target_url("http://plugin.local/", "orders", None),
        "http://plugin.local/orders"
    );
    assert_eq!(
        target_url("http://plugin.local", "orders", Some("a=1&b=2")),
        "http://plugin.local/orders?a=1&b=2"
    );
    assert_eq!(
        target_url("http://plugin.local", "", None),
        "http://plugin.local/"
    );
    assert_eq!(
        target_url("http://plugin.local", "a/b/c", Some("")),
        "http://plugin.local/a/b/c"
    );
}

#[tokio::test]
async fn forwards_the_plugin_name_from_the_path() {
    let upstream = FakeUpstream::answering(reply(StatusCode::OK, ""));
    let plugins = registry(
        r#"{"plugins":[{"name":"one","url":"http://one.local"},{"name":"two","url":"http://two.local"}]}"#,
    );
    let concrete: Arc<FakeUpstream> = Arc::clone(&upstream);
    let app = router(state(plugins, concrete, None));
    let request = authorized(get_request("/plugin-proxy/two/deep/path"), master_key());
    let (_status, _headers, _body) = send(app, request).await;
    let sent = upstream.sent();
    assert_eq!(sent[0].url, "http://two.local/deep/path");
    assert_eq!(sent[0].headers["x-litellm-user-role"], "proxy_admin");
}
