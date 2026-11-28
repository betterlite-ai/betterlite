use axum::http::StatusCode;
use betterlite_config::sections::mcp_management_mode::ManagementMode;

use super::support::{
    Harness, MEMBER, NO_USER, REGISTRY_SERVER, SERVER_BARE, SERVER_ONE, SERVER_TWO, SourcesAnswer,
    call, context, get,
};
use crate::mcp_user_env_routes::router::USER_ENV_VARS_STATUS_PATH;
use crate::mcp_user_env_routes::types::MCPUserEnvVarsStatus;

const DATABASE_DETAIL: &str = "Database not connected. Connect a database to your proxy";

#[tokio::test]
async fn list_answers_nothing_without_a_user_id() {
    let harness = Harness::new();
    let reply = call(
        harness.router(),
        get(USER_ENV_VARS_STATUS_PATH, Some(&context(NO_USER))),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::OK);
    let statuses: Vec<MCPUserEnvVarsStatus> = reply.json();
    assert!(statuses.is_empty());
    assert!(harness.journal().is_empty());
}

#[tokio::test]
async fn list_without_an_auth_extension_answers_nothing() {
    let harness = Harness::new();
    let reply = call(harness.router(), get(USER_ENV_VARS_STATUS_PATH, None)).await;
    assert_eq!(reply.status(), StatusCode::OK);
    let statuses: Vec<MCPUserEnvVarsStatus> = reply.json();
    assert!(statuses.is_empty());
    assert!(harness.journal().is_empty());
}

#[tokio::test]
async fn list_requires_the_database_first() {
    let harness = Harness::new().without_database();
    let reply = call(
        harness.router(),
        get(USER_ENV_VARS_STATUS_PATH, Some(&context(NO_USER))),
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
async fn list_answers_nothing_without_accessible_servers() {
    let harness = Harness::new();
    let reply = call(
        harness.router(),
        get(USER_ENV_VARS_STATUS_PATH, Some(&context(MEMBER))),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::OK);
    let statuses: Vec<MCPUserEnvVarsStatus> = reply.json();
    assert!(statuses.is_empty());
    assert_eq!(harness.journal(), ["sources:contexts"]);
}

#[tokio::test]
async fn list_reads_all_rows_in_one_bulk_call_and_keeps_the_order() {
    let harness = Harness::new().answering_from(SourcesAnswer::Servers(vec![
        SERVER_TWO,
        SERVER_ONE,
        SERVER_BARE,
    ]));
    harness
        .store()
        .seed("user-1", "srv-1", &[("TOKEN", "secret")]);
    harness
        .store()
        .seed("user-2", "srv-2", &[("OTHER", "other-secret")]);
    let reply = call(
        harness.router(),
        get(USER_ENV_VARS_STATUS_PATH, Some(&context(MEMBER))),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::OK);
    let statuses: Vec<MCPUserEnvVarsStatus> = reply.json();
    let ids: Vec<&str> = statuses
        .iter()
        .map(|status| status.server_id.as_str())
        .collect();
    assert_eq!(ids, ["srv-2", "srv-1"]);
    assert_eq!(statuses[0].required[0].name, "OTHER");
    assert!(!statuses[0].required[0].is_set);
    assert_eq!(statuses[0].missing_count, 1);
    assert!(statuses[1].required[0].is_set);
    assert_eq!(statuses[1].missing_count, 0);
    assert_eq!(
        harness.journal(),
        [
            "sources:contexts",
            "sources:tables",
            "bulk:user-1:srv-2,srv-1,srv-3"
        ]
    );
}

#[tokio::test]
async fn list_uses_the_registry_in_view_all_mode() {
    let harness = Harness::new()
        .in_mode(ManagementMode::ViewAll)
        .with_registry_servers(vec![REGISTRY_SERVER])
        .answering_from(SourcesAnswer::Servers(vec![SERVER_ONE]));
    let reply = call(
        harness.router(),
        get(USER_ENV_VARS_STATUS_PATH, Some(&context(MEMBER))),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::OK);
    let statuses: Vec<MCPUserEnvVarsStatus> = reply.json();
    assert!(statuses.is_empty());
    assert_eq!(harness.journal(), ["bulk:user-1:srv-9"]);
}

#[tokio::test]
async fn list_context_failure_is_the_internal_error() {
    let harness = Harness::new().answering_from(SourcesAnswer::ContextsFail);
    let reply = call(
        harness.router(),
        get(USER_ENV_VARS_STATUS_PATH, Some(&context(MEMBER))),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.error().message, "Internal server error");
    assert_eq!(harness.journal(), ["sources:contexts"]);
}

#[tokio::test]
async fn list_table_failure_is_the_internal_error() {
    let harness = Harness::new().answering_from(SourcesAnswer::TablesFail);
    let reply = call(
        harness.router(),
        get(USER_ENV_VARS_STATUS_PATH, Some(&context(MEMBER))),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.error().kind, "internal_server_error");
    assert_eq!(harness.journal(), ["sources:contexts", "sources:tables"]);
}

#[tokio::test]
async fn list_bulk_failure_is_the_internal_error() {
    let harness = Harness::new().answering_from(SourcesAnswer::Servers(vec![SERVER_ONE]));
    harness.store().fail_bulk_read();
    let reply = call(
        harness.router(),
        get(USER_ENV_VARS_STATUS_PATH, Some(&context(MEMBER))),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        harness.journal(),
        ["sources:contexts", "sources:tables", "bulk:user-1:srv-1"]
    );
}
