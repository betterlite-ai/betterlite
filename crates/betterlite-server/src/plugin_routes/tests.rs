#![cfg(test)]

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};

use super::claim::{SALT_VARIABLE, verify_plugin_session_claim};
use super::config::PluginRegistry;
use super::router::router;
use super::test_support::{
    FakeUpstream, PLUGIN, SALT, authorized, json, key_caller, master_key, registry, reply, send,
    state, text,
};

fn platform_registry() -> Arc<PluginRegistry> {
    registry(
        r#"{"plugins":[{"name":"alpha","display_name":"Alpha","url":"http://alpha"},{"name":"litellm-platform-plugin","url":"http://plugin.local","plugin_key":"sk-plugin"}]}"#,
    )
}

fn application(upstream: Arc<FakeUpstream>) -> Router {
    router(state(platform_registry(), upstream, None))
}

fn get_request(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

#[tokio::test]
async fn oneshot_lists_the_configured_plugins() {
    let app = application(FakeUpstream::answering(reply(StatusCode::OK, "")));
    let (_status, _headers, body) =
        send(app, authorized(get_request("/api/plugins"), master_key())).await;
    let entries = json(&body);
    assert_eq!(entries.as_array().unwrap().len(), 2);
    assert_eq!(entries[0]["name"], "alpha");
    assert_eq!(entries[0]["display_name"], "Alpha");
    assert_eq!(entries[1]["name"], PLUGIN);
    assert!(entries[1].get("plugin_key").is_none());
    assert!(!text(&body).contains("sk-plugin"));
}

#[tokio::test]
async fn oneshot_issues_a_claim_for_the_platform_plugin() {
    temp_env::async_with_vars([(SALT_VARIABLE, Some(SALT))], async {
        let app = application(FakeUpstream::answering(reply(StatusCode::OK, "")));
        let request = authorized(
            get_request("/api/plugins/auth-token?plugin_name=litellm-platform-plugin"),
            master_key(),
        );
        let (status, _headers, body) = send(app, request).await;
        assert_eq!(status, StatusCode::OK);
        let token = json(&body)["session_claim"].as_str().unwrap().to_owned();
        let claim = verify_plugin_session_claim(PLUGIN, &token).unwrap();
        assert_eq!(claim.plugin, PLUGIN);
        assert_eq!(claim.user_id, "default_user_id");
        assert_eq!(claim.user_role, "proxy_admin");
    })
    .await;
}

#[tokio::test]
async fn oneshot_proxies_a_request_to_the_plugin() {
    let upstream = FakeUpstream::answering(reply(StatusCode::OK, "pong"));
    let app = application(Arc::clone(&upstream));
    let request = authorized(
        Request::builder()
            .method(Method::POST)
            .uri(format!("/plugin-proxy/{PLUGIN}/echo?source=test"))
            .body(Body::from("ping"))
            .unwrap(),
        master_key(),
    );
    let (status, _headers, body) = send(app, request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(text(&body), "pong");
    let sent = upstream.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, Method::POST);
    assert_eq!(sent[0].url, "http://plugin.local/echo?source=test");
    assert_eq!(sent[0].body, b"ping".to_vec());
    assert_eq!(sent[0].headers["authorization"], "Bearer sk-plugin");
    assert_eq!(sent[0].headers["x-litellm-user-id"], "default_user_id");
}

#[tokio::test]
async fn oneshot_rejects_a_non_admin_on_the_proxy() {
    let upstream = FakeUpstream::answering(reply(StatusCode::OK, ""));
    let app = application(Arc::clone(&upstream));
    let request = authorized(
        get_request("/plugin-proxy/litellm-platform-plugin/path"),
        key_caller(r#"{"user_id":"u9","user_role":"internal_user"}"#),
    );
    let (status, _headers, body) = send(app, request).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        text(&body),
        "Plugin proxy access requires proxy_admin role."
    );
    assert!(upstream.sent().is_empty());
}

#[tokio::test]
async fn oneshot_answers_503_without_a_salt() {
    temp_env::async_with_vars([(SALT_VARIABLE, None::<&str>)], async {
        let app = application(FakeUpstream::answering(reply(StatusCode::OK, "")));
        let request = authorized(get_request("/api/plugins/auth-token"), master_key());
        let (status, _headers, body) = send(app, request).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            json(&body)["detail"],
            "LITELLM_SALT_KEY is not configured; plugin iframe auth unavailable."
        );
    })
    .await;
}

#[tokio::test]
async fn oneshot_hides_the_proxy_without_a_trailing_slash() {
    let upstream = FakeUpstream::answering(reply(StatusCode::OK, ""));
    let app = application(Arc::clone(&upstream));
    let request = authorized(
        get_request("/plugin-proxy/litellm-platform-plugin"),
        master_key(),
    );
    let (status, _headers, _body) = send(app, request).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(upstream.sent().is_empty());
}
