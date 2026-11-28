use axum::http::StatusCode;

use super::support::{
    ADMIN, Harness, MEMBER, NO_USER, call, context, expected, names, plain, post, server_uri,
};
use crate::mcp_user_env_routes::types::MCPUserEnvVarsStatus;

const DATABASE_DETAIL: &str = "Database not connected. Connect a database to your proxy";

#[tokio::test]
async fn post_stores_the_declared_value() {
    let harness = Harness::new();
    let reply = call(
        harness.router(),
        post(
            &server_uri("srv-1"),
            Some(&context(MEMBER)),
            r#"{"values":{"TOKEN":"secret"}}"#,
        ),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::OK);
    let status: MCPUserEnvVarsStatus = reply.json();
    assert!(status.required[0].is_set);
    assert_eq!(status.missing_count, 0);
    assert_eq!(
        harness.journal(),
        [
            "lookup:srv-1",
            "access:srv-1",
            "merge:user-1:srv-1",
            "cache:user-1:srv-1"
        ]
    );
    let merges = harness.store().merges();
    assert_eq!(merges.len(), 1);
    assert_eq!(merges[0].user_id, "user-1");
    assert_eq!(merges[0].server_id, "srv-1");
    assert_eq!(plain(&merges[0].updates), expected(&[("TOKEN", "secret")]));
    assert_eq!(merges[0].allowed_names, names(&["TOKEN"]));
    assert_eq!(
        harness
            .store()
            .row("user-1", "srv-1")
            .map(|row| plain(&row)),
        Some(expected(&[("TOKEN", "secret")]))
    );
}

#[tokio::test]
async fn post_overwrites_a_stored_value() {
    let harness = Harness::new();
    harness.store().seed("user-1", "srv-1", &[("TOKEN", "old")]);
    let reply = call(
        harness.router(),
        post(
            &server_uri("srv-1"),
            Some(&context(MEMBER)),
            r#"{"values":{"TOKEN":"new"}}"#,
        ),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::OK);
    assert_eq!(
        harness
            .store()
            .row("user-1", "srv-1")
            .map(|row| plain(&row)),
        Some(expected(&[("TOKEN", "new")]))
    );
}

#[tokio::test]
async fn post_keeps_a_stored_value_when_the_submitted_one_is_blank() {
    let harness = Harness::new();
    harness.store().seed("user-1", "srv-1", &[("TOKEN", "old")]);
    let reply = call(
        harness.router(),
        post(
            &server_uri("srv-1"),
            Some(&context(MEMBER)),
            r#"{"values":{"TOKEN":""}}"#,
        ),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::OK);
    let status: MCPUserEnvVarsStatus = reply.json();
    assert!(status.required[0].is_set);
    assert!(plain(&harness.store().merges()[0].updates).is_empty());
    assert_eq!(
        harness
            .store()
            .row("user-1", "srv-1")
            .map(|row| plain(&row)),
        Some(expected(&[("TOKEN", "old")]))
    );
}

#[tokio::test]
async fn post_drops_an_undeclared_name() {
    let harness = Harness::new();
    let reply = call(
        harness.router(),
        post(
            &server_uri("srv-1"),
            Some(&context(MEMBER)),
            r#"{"values":{"TOKEN":"kept","EVIL":"dropped"}}"#,
        ),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::OK);
    assert_eq!(
        plain(&harness.store().merges()[0].updates),
        expected(&[("TOKEN", "kept")])
    );
    let row = harness.store().row("user-1", "srv-1").unwrap();
    assert!(!plain(&row).contains_key("EVIL"));
    assert!(!reply.text().contains("dropped"));
}

#[tokio::test]
async fn post_prunes_a_retired_variable() {
    let harness = Harness::new();
    harness
        .store()
        .seed("user-1", "srv-1", &[("TOKEN", "old"), ("RETIRED", "gone")]);
    let reply = call(
        harness.router(),
        post(
            &server_uri("srv-1"),
            Some(&context(MEMBER)),
            r#"{"values":{}}"#,
        ),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::OK);
    assert_eq!(harness.store().merges()[0].allowed_names, names(&["TOKEN"]));
    assert_eq!(
        harness
            .store()
            .row("user-1", "srv-1")
            .map(|row| plain(&row)),
        Some(expected(&[("TOKEN", "old")]))
    );
}

#[tokio::test]
async fn post_leaves_the_cache_alone_when_the_merge_fails() {
    let harness = Harness::new();
    harness.store().fail_write();
    let reply = call(
        harness.router(),
        post(
            &server_uri("srv-1"),
            Some(&context(MEMBER)),
            r#"{"values":{"TOKEN":"secret"}}"#,
        ),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.error().message, "Internal server error");
    assert_eq!(
        harness.journal(),
        ["lookup:srv-1", "access:srv-1", "merge:user-1:srv-1"]
    );
    assert!(harness.store().row("user-1", "srv-1").is_none());
}

#[tokio::test]
async fn post_without_a_database_answers_the_detail_500() {
    let harness = Harness::new().without_database();
    let reply = call(
        harness.router(),
        post(
            &server_uri("srv-1"),
            Some(&context(MEMBER)),
            r#"{"values":{"TOKEN":"secret"}}"#,
        ),
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
async fn post_without_a_user_id_answers_the_400() {
    let harness = Harness::new();
    let reply = call(
        harness.router(),
        post(
            &server_uri("srv-1"),
            Some(&context(NO_USER)),
            r#"{"values":{"TOKEN":"secret"}}"#,
        ),
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
async fn post_unknown_server_is_forbidden_for_a_member() {
    let harness = Harness::new().unknown_server();
    let reply = call(
        harness.router(),
        post(
            &server_uri("srv-1"),
            Some(&context(MEMBER)),
            r#"{"values":{"TOKEN":"secret"}}"#,
        ),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::FORBIDDEN);
    assert_eq!(harness.journal(), ["lookup:srv-1"]);
    assert!(harness.store().row("user-1", "srv-1").is_none());
}

#[tokio::test]
async fn post_unknown_server_is_not_found_for_an_admin() {
    let harness = Harness::new().unknown_server();
    let reply = call(
        harness.router(),
        post(
            &server_uri("srv-1"),
            Some(&context(ADMIN)),
            r#"{"values":{"TOKEN":"secret"}}"#,
        ),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        reply.detail().get("error").map(String::as_str),
        Some("MCP Server srv-1 not found")
    );
    assert!(harness.store().row("admin-1", "srv-1").is_none());
}
