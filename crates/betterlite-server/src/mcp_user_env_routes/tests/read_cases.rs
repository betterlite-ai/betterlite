use axum::http::StatusCode;

use super::support::{
    ADMIN, Harness, MEMBER, NO_USER, OTHER_MEMBER, call, context, get, server_uri,
};
use crate::mcp_user_env_routes::types::{MCPUserEnvVarSpec, MCPUserEnvVarsStatus};

const DATABASE_DETAIL: &str = "Database not connected. Connect a database to your proxy";

#[tokio::test]
async fn get_answers_the_callers_status() {
    let harness = Harness::new();
    harness
        .store()
        .seed("user-1", "srv-1", &[("TOKEN", "secret")]);
    let reply = call(
        harness.router(),
        get(&server_uri("srv-1"), Some(&context(MEMBER))),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::OK);
    let status: MCPUserEnvVarsStatus = reply.json();
    assert_eq!(
        status.required,
        [MCPUserEnvVarSpec {
            name: "TOKEN".to_owned(),
            description: Some("Paste the token".to_owned()),
            is_set: true,
        }]
    );
    assert_eq!(status.missing_count, 0);
    assert_eq!(status.server_name.as_deref(), Some("Server One"));
    assert_eq!(status.alias.as_deref(), Some("one"));
    assert_eq!(
        status.setup_url.as_deref(),
        Some("https://proxy.example/ui/mcp-servers?fill_env_vars=srv-1")
    );
    assert_eq!(
        harness.journal(),
        ["lookup:srv-1", "access:srv-1", "get:user-1:srv-1"]
    );
}

#[tokio::test]
async fn get_reads_only_the_callers_row() {
    let harness = Harness::new();
    harness
        .store()
        .seed("user-2", "srv-1", &[("TOKEN", "secret")]);
    let reply = call(
        harness.router(),
        get(&server_uri("srv-1"), Some(&context(MEMBER))),
    )
    .await;
    let status: MCPUserEnvVarsStatus = reply.json();
    assert!(!status.required[0].is_set);
    assert_eq!(status.missing_count, 1);
    assert_eq!(
        harness.journal(),
        ["lookup:srv-1", "access:srv-1", "get:user-1:srv-1"]
    );
}

#[tokio::test]
async fn get_without_a_database_answers_the_detail_500() {
    let harness = Harness::new().without_database();
    let reply = call(
        harness.router(),
        get(&server_uri("srv-1"), Some(&context(MEMBER))),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        reply.detail().get("error").map(String::as_str),
        Some(DATABASE_DETAIL)
    );
    assert!(harness.journal().is_empty());
}

#[tokio::test]
async fn get_without_a_user_id_answers_the_400() {
    let harness = Harness::new();
    let reply = call(
        harness.router(),
        get(&server_uri("srv-1"), Some(&context(NO_USER))),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        reply.detail().get("error").map(String::as_str),
        Some("User ID not found in token")
    );
    assert!(harness.journal().is_empty());
}

#[tokio::test]
async fn get_without_an_auth_extension_answers_the_400() {
    let harness = Harness::new();
    let reply = call(harness.router(), get(&server_uri("srv-1"), None)).await;
    assert_eq!(reply.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        reply.detail().get("error").map(String::as_str),
        Some("User ID not found in token")
    );
    assert!(harness.journal().is_empty());
}

#[tokio::test]
async fn a_missing_database_wins_over_a_missing_user_id() {
    let harness = Harness::new().without_database();
    let reply = call(
        harness.router(),
        get(&server_uri("srv-1"), Some(&context(NO_USER))),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        reply.detail().get("error").map(String::as_str),
        Some(DATABASE_DETAIL)
    );
    assert!(harness.journal().is_empty());
}

#[tokio::test]
async fn a_missing_database_wins_over_an_unknown_server() {
    let harness = Harness::new().without_database().unknown_server();
    let reply = call(
        harness.router(),
        get(&server_uri("srv-1"), Some(&context(ADMIN))),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        reply.detail().get("error").map(String::as_str),
        Some(DATABASE_DETAIL)
    );
    assert!(harness.journal().is_empty());
}

#[tokio::test]
async fn get_unknown_server_is_not_found_for_an_admin() {
    let harness = Harness::new().unknown_server();
    let reply = call(
        harness.router(),
        get(&server_uri("srv-1"), Some(&context(ADMIN))),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        reply.detail().get("error").map(String::as_str),
        Some("MCP Server srv-1 not found")
    );
    assert_eq!(harness.journal(), ["lookup:srv-1"]);
}

#[tokio::test]
async fn get_unknown_server_is_forbidden_for_a_member() {
    let harness = Harness::new().unknown_server();
    let reply = call(
        harness.router(),
        get(&server_uri("srv-1"), Some(&context(MEMBER))),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        reply.detail().get("error").map(String::as_str),
        Some(
            "User does not have permission to access mcp server with id srv-1. You can only manage mcp servers that you have access to."
        )
    );
    assert_eq!(harness.journal(), ["lookup:srv-1"]);
}

#[tokio::test]
async fn get_denied_server_is_forbidden_for_a_member() {
    let harness = Harness::new().denying_access();
    let reply = call(
        harness.router(),
        get(&server_uri("srv-1"), Some(&context(MEMBER))),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        reply.detail().get("error").map(String::as_str),
        Some(
            "User does not have permission to access mcp server with id srv-1. You can only manage mcp servers that you have access to."
        )
    );
    assert_eq!(harness.journal(), ["lookup:srv-1", "access:srv-1"]);
}

#[tokio::test]
async fn get_lets_an_admin_skip_the_access_check() {
    let harness = Harness::new().denying_access();
    let reply = call(
        harness.router(),
        get(&server_uri("srv-1"), Some(&context(ADMIN))),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::OK);
    assert_eq!(harness.journal(), ["lookup:srv-1", "get:admin-1:srv-1"]);
}

#[tokio::test]
async fn get_an_access_failure_is_the_internal_error() {
    let harness = Harness::new().failing_access();
    let reply = call(
        harness.router(),
        get(&server_uri("srv-1"), Some(&context(MEMBER))),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.error().message, "Internal server error");
    assert_eq!(reply.error().kind, "internal_server_error");
    assert_eq!(harness.journal(), ["lookup:srv-1", "access:srv-1"]);
}

#[tokio::test]
async fn get_a_lookup_failure_is_the_internal_error() {
    let harness = Harness::new().failing_lookup();
    let reply = call(
        harness.router(),
        get(&server_uri("srv-1"), Some(&context(MEMBER))),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.error().message, "Internal server error");
    assert_eq!(harness.journal(), ["lookup:srv-1"]);
}

#[tokio::test]
async fn get_reports_a_path_only_setup_url_without_a_base() {
    let harness = Harness::new().without_base_url();
    let reply = call(
        harness.router(),
        get(&server_uri("srv-1"), Some(&context(MEMBER))),
    )
    .await;
    let status: MCPUserEnvVarsStatus = reply.json();
    assert_eq!(
        status.setup_url.as_deref(),
        Some("/ui/mcp-servers?fill_env_vars=srv-1")
    );
}

#[tokio::test]
async fn another_member_reads_their_own_row() {
    let harness = Harness::new();
    harness
        .store()
        .seed("user-1", "srv-1", &[("TOKEN", "secret")]);
    let reply = call(
        harness.router(),
        get(&server_uri("srv-1"), Some(&context(OTHER_MEMBER))),
    )
    .await;
    let status: MCPUserEnvVarsStatus = reply.json();
    assert!(!status.required[0].is_set);
    assert_eq!(
        harness.journal(),
        ["lookup:srv-1", "access:srv-1", "get:user-2:srv-1"]
    );
}
