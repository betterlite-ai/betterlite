#![cfg(test)]

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::get as route_get;
use betterlite_auth::auth_identity::roles::Role;

use super::{caller_identity, list_plugins, plugin_auth_token};
use crate::plugin_routes::claim::{SALT_VARIABLE, verify_plugin_session_claim};
use crate::plugin_routes::config::PluginRegistry;
use crate::plugin_routes::proxy::PluginState;
use crate::plugin_routes::test_support::{
    FakeUpstream, PLUGIN, SALT, authorized, caller, json, key_caller, master_key, registry, reply,
    send, state, text, user_caller,
};

fn application(state: PluginState) -> Router {
    Router::new()
        .route("/api/plugins", route_get(list_plugins))
        .route("/api/plugins/auth-token", route_get(plugin_auth_token))
        .with_state(state)
}

fn platform_registry() -> Arc<PluginRegistry> {
    registry(
        r#"{"plugins":[{"name":"litellm-platform-plugin","display_name":"Platform","url":"http://plugin.local"}]}"#,
    )
}

fn idle_state() -> PluginState {
    state(
        platform_registry(),
        FakeUpstream::answering(reply(StatusCode::OK, "")),
        None,
    )
}

fn get_request(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

#[tokio::test]
async fn list_returns_the_registered_plugins_in_order() {
    let app = application(state(
        registry(
            r#"{"plugins":[{"name":"alpha","display_name":"Alpha","url":"http://alpha"},{"name":"beta","url":"http://beta"}]}"#,
        ),
        FakeUpstream::answering(reply(StatusCode::OK, "")),
        None,
    ));
    let (_status, _headers, body) =
        send(app, authorized(get_request("/api/plugins"), master_key())).await;
    let entries = json(&body);
    assert_eq!(entries.as_array().unwrap().len(), 2);
    assert_eq!(entries[0]["name"], "alpha");
    assert_eq!(entries[0]["display_name"], "Alpha");
    assert_eq!(entries[0]["url"], "http://alpha");
    assert_eq!(entries[1]["display_name"], "beta");
}

#[tokio::test]
async fn list_hides_the_plugin_key() {
    let app = application(state(
        registry(r#"{"plugins":[{"name":"alpha","url":"http://alpha","plugin_key":"sk-plugin"}]}"#),
        FakeUpstream::answering(reply(StatusCode::OK, "")),
        None,
    ));
    let (_status, _headers, body) =
        send(app, authorized(get_request("/api/plugins"), master_key())).await;
    let entries = json(&body);
    assert!(entries[0].get("plugin_key").is_none());
    assert!(!text(&body).contains("sk-plugin"));
}

#[tokio::test]
async fn list_requires_a_caller() {
    let app = application(idle_state());
    let (status, _headers, _body) = send(app, get_request("/api/plugins")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn auth_token_requires_a_caller() {
    let app = application(idle_state());
    let (status, _headers, _body) = send(app, get_request("/api/plugins/auth-token")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn auth_token_answers_503_when_the_salt_is_unset() {
    temp_env::async_with_vars([(SALT_VARIABLE, None::<&str>)], async {
        let app = application(idle_state());
        let request = authorized(
            get_request("/api/plugins/auth-token?plugin_name=missing"),
            master_key(),
        );
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
async fn auth_token_answers_503_when_the_salt_is_empty() {
    temp_env::async_with_vars([(SALT_VARIABLE, Some(""))], async {
        let app = application(idle_state());
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
async fn auth_token_answers_404_for_an_unregistered_plugin() {
    temp_env::async_with_vars([(SALT_VARIABLE, Some(SALT))], async {
        let app = application(idle_state());
        let request = authorized(
            get_request("/api/plugins/auth-token?plugin_name=missing"),
            master_key(),
        );
        let (status, _headers, body) = send(app, request).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(json(&body)["detail"], "Plugin 'missing' is not registered.");
    })
    .await;
}

#[tokio::test]
async fn auth_token_issues_a_claim_for_the_master_key() {
    temp_env::async_with_vars([(SALT_VARIABLE, Some(SALT))], async {
        let app = application(idle_state());
        let request = authorized(get_request("/api/plugins/auth-token"), master_key());
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
async fn auth_token_uses_the_key_identity() {
    temp_env::async_with_vars([(SALT_VARIABLE, Some(SALT))], async {
        let app = application(idle_state());
        let request = authorized(
            get_request("/api/plugins/auth-token"),
            key_caller(r#"{"user_id":"u9","user_role":"internal_user"}"#),
        );
        let (status, _headers, body) = send(app, request).await;
        assert_eq!(status, StatusCode::OK);
        let token = json(&body)["session_claim"].as_str().unwrap().to_owned();
        let claim = verify_plugin_session_claim(PLUGIN, &token).unwrap();
        assert_eq!(claim.user_id, "u9");
        assert_eq!(claim.user_role, "internal_user");
    })
    .await;
}

fn claim_role_for(roles: Vec<Role>) -> String {
    let context = caller("key-owner", roles);
    caller_identity(&context.principal)
        .expect("the caller identity is read")
        .user_role
        .expect("the caller has a role")
}

#[tokio::test]
async fn auth_token_uses_the_user_record_id_when_the_caller_has_no_key() {
    temp_env::async_with_vars([(SALT_VARIABLE, Some(SALT))], async {
        let app = application(idle_state());
        let request = authorized(
            get_request("/api/plugins/auth-token"),
            user_caller("jwt-user", vec![Role::PlatformAdmin], "u-jwt"),
        );
        let (status, _headers, body) = send(app, request).await;
        assert_eq!(status, StatusCode::OK);
        let token = json(&body)["session_claim"].as_str().unwrap().to_owned();
        let claim = verify_plugin_session_claim(PLUGIN, &token).unwrap();
        assert_eq!(claim.user_id, "u-jwt");
        assert_eq!(claim.user_role, "proxy_admin");
    })
    .await;
}

#[test]
fn auth_token_falls_back_to_the_platform_admin_role() {
    assert_eq!(claim_role_for(vec![Role::PlatformAdmin]), "proxy_admin");
}

#[test]
fn auth_token_falls_back_to_the_platform_viewer_role() {
    assert_eq!(
        claim_role_for(vec![Role::PlatformViewer]),
        "proxy_admin_viewer"
    );
}

#[test]
fn auth_token_falls_back_to_the_org_admin_role() {
    assert_eq!(claim_role_for(vec![Role::OrgAdmin]), "org_admin");
}

#[test]
fn auth_token_falls_back_to_the_internal_user_role() {
    assert_eq!(claim_role_for(Vec::new()), "internal_user");
}

#[tokio::test]
async fn auth_token_prefers_the_key_role_over_the_principal_roles() {
    temp_env::async_with_vars([(SALT_VARIABLE, Some(SALT))], async {
        let mut context = key_caller(r#"{"user_id":"u9","user_role":"internal_user"}"#);
        context.principal.roles = vec![Role::PlatformAdmin];
        let app = application(idle_state());
        let request = authorized(get_request("/api/plugins/auth-token"), context);
        let (status, _headers, body) = send(app, request).await;
        assert_eq!(status, StatusCode::OK);
        let token = json(&body)["session_claim"].as_str().unwrap().to_owned();
        let claim = verify_plugin_session_claim(PLUGIN, &token).unwrap();
        assert_eq!(claim.user_role, "internal_user");
    })
    .await;
}
