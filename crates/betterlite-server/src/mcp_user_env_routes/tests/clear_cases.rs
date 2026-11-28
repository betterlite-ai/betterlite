use axum::http::StatusCode;

use super::support::{Harness, MEMBER, NO_USER, call, context, delete, server_uri};
use crate::mcp_user_env_routes::types::MCPUserEnvVarsStatus;

const DATABASE_DETAIL: &str = "Database not connected. Connect a database to your proxy";

#[tokio::test]
async fn delete_clears_the_callers_row() {
    let harness = Harness::new();
    harness
        .store()
        .seed("user-1", "srv-1", &[("TOKEN", "secret")]);
    let reply = call(
        harness.router(),
        delete(&server_uri("srv-1"), Some(&context(MEMBER))),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::OK);
    let status: MCPUserEnvVarsStatus = reply.json();
    assert!(!status.required[0].is_set);
    assert_eq!(status.missing_count, 1);
    let required_count = u64::try_from(status.required.len()).unwrap();
    assert_eq!(status.missing_count, required_count);
    assert!(harness.store().row("user-1", "srv-1").is_none());
    assert_eq!(
        harness.journal(),
        [
            "lookup:srv-1",
            "access:srv-1",
            "delete:user-1:srv-1",
            "cache:user-1:srv-1"
        ]
    );
}

#[tokio::test]
async fn delete_does_not_re_read_the_store() {
    let harness = Harness::new();
    harness
        .store()
        .seed("user-1", "srv-1", &[("TOKEN", "secret")]);
    harness.store().fail_reads();
    let reply = call(
        harness.router(),
        delete(&server_uri("srv-1"), Some(&context(MEMBER))),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::OK);
    let status: MCPUserEnvVarsStatus = reply.json();
    assert!(!status.required[0].is_set);
    assert!(
        !harness
            .journal()
            .iter()
            .any(|event| event.starts_with("get:"))
    );
}

#[tokio::test]
async fn delete_of_a_missing_row_succeeds() {
    let harness = Harness::new();
    let reply = call(
        harness.router(),
        delete(&server_uri("srv-1"), Some(&context(MEMBER))),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::OK);
    let status: MCPUserEnvVarsStatus = reply.json();
    assert!(!status.required[0].is_set);
    assert_eq!(
        harness.journal(),
        [
            "lookup:srv-1",
            "access:srv-1",
            "delete:user-1:srv-1",
            "cache:user-1:srv-1"
        ]
    );
}

#[tokio::test]
async fn delete_leaves_the_cache_alone_when_the_delete_fails() {
    let harness = Harness::new();
    harness.store().fail_clear();
    let reply = call(
        harness.router(),
        delete(&server_uri("srv-1"), Some(&context(MEMBER))),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.error().message, "Internal server error");
    assert_eq!(
        harness.journal(),
        ["lookup:srv-1", "access:srv-1", "delete:user-1:srv-1"]
    );
}

#[tokio::test]
async fn delete_without_a_database_answers_the_detail_500() {
    let harness = Harness::new().without_database();
    let reply = call(
        harness.router(),
        delete(&server_uri("srv-1"), Some(&context(MEMBER))),
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
async fn delete_without_a_user_id_answers_the_400() {
    let harness = Harness::new();
    let reply = call(
        harness.router(),
        delete(&server_uri("srv-1"), Some(&context(NO_USER))),
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
async fn delete_unknown_server_is_forbidden_for_a_member() {
    let harness = Harness::new().unknown_server();
    let reply = call(
        harness.router(),
        delete(&server_uri("srv-1"), Some(&context(MEMBER))),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::FORBIDDEN);
    assert_eq!(harness.journal(), ["lookup:srv-1"]);
}
