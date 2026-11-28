use axum::http::StatusCode;

use super::support::{Harness, MEMBER, SERVER_ANON, call, context, get, post, put, server_uri};
use crate::mcp_user_env_routes::router::{SERVER_ENV_VARS_PATH, USER_ENV_VARS_STATUS_PATH};
use crate::mcp_user_env_routes::types::{MCPUserEnvVarSpec, MCPUserEnvVarsStatus};

#[test]
fn the_routes_use_the_litellm_paths() {
    assert_eq!(
        SERVER_ENV_VARS_PATH,
        "/v1/mcp/server/{server_id}/user-env-vars"
    );
    assert_eq!(USER_ENV_VARS_STATUS_PATH, "/v1/mcp/user-env-vars/status");
}

#[tokio::test]
async fn the_litellm_paths_reach_the_handlers() {
    let harness = Harness::new();
    let app = harness.router();
    let status = call(
        app.clone(),
        get("/v1/mcp/server/srv-1/user-env-vars", Some(&context(MEMBER))),
    )
    .await;
    assert_eq!(status.status(), StatusCode::OK);
    let list = call(
        app.clone(),
        get("/v1/mcp/user-env-vars/status", Some(&context(MEMBER))),
    )
    .await;
    assert_eq!(list.status(), StatusCode::OK);
}

#[tokio::test]
async fn the_server_path_answers_method_not_allowed_for_put() {
    let harness = Harness::new();
    let reply = call(
        harness.router(),
        put(&server_uri("srv-1"), Some(&context(MEMBER)), "{}"),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn the_status_path_answers_method_not_allowed_for_post() {
    let harness = Harness::new();
    let reply = call(
        harness.router(),
        post(USER_ENV_VARS_STATUS_PATH, Some(&context(MEMBER)), "{}"),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn an_unknown_path_is_not_found() {
    let harness = Harness::new();
    let reply = call(
        harness.router(),
        get(
            "/v1/mcp/server/srv-1/user-env-vars/extra",
            Some(&context(MEMBER)),
        ),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_wire_types_use_litellm_field_names_and_defaults() {
    let spec = MCPUserEnvVarSpec {
        name: "TOKEN".to_owned(),
        description: None,
        is_set: false,
    };
    assert_eq!(
        serde_json::to_string(&spec).unwrap(),
        r#"{"name":"TOKEN","description":null,"is_set":false}"#
    );
    let status = MCPUserEnvVarsStatus {
        server_id: "srv-1".to_owned(),
        server_name: None,
        alias: None,
        required: Vec::new(),
        missing_count: 0,
        setup_url: None,
    };
    assert_eq!(
        serde_json::to_string(&status).unwrap(),
        r#"{"server_id":"srv-1","server_name":null,"alias":null,"required":[],"missing_count":0,"setup_url":null}"#
    );
}

#[tokio::test]
async fn the_status_response_carries_the_null_identity_fields() {
    let harness = Harness::new().for_server(SERVER_ANON);
    let reply = call(
        harness.router(),
        get(&server_uri("srv-1"), Some(&context(MEMBER))),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::OK);
    let text = reply.text();
    assert!(text.contains(r#""server_name":null"#));
    assert!(text.contains(r#""alias":null"#));
    assert!(text.contains(r#""required":[]"#));
    assert!(text.contains(r#""missing_count":0"#));
    assert!(text.contains(r#""setup_url":null"#));
}
