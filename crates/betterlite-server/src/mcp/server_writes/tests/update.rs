use std::collections::BTreeMap;

use betterlite_db::mcp::table_access::types::IdentifierField;
use betterlite_db::mcp_stored_contracts::MCPTransport;

use super::support::cipher::{SALT, cipher};
use super::support::database::{SERVER_COUNT_SQL, database};
use super::support::fake::RecordingInvalidation;
use super::support::{request, update_payload};
use crate::mcp::server_writes::create::create_mcp_server;
use crate::mcp::server_writes::delete::delete_mcp_server;
use crate::mcp::server_writes::store::PostgresServerWrites;
use crate::mcp::server_writes::update::update_mcp_server;
use crate::mcp::server_writes::{ServerWriteError, UpdatedServer};

const ALIAS_SQL: &str =
    r#"SELECT "alias"::text FROM "LiteLLM_MCPServerTable" WHERE "server_id" = $1"#;
const EXISTS_SQL: &str = r#"SELECT 1::text FROM "LiteLLM_MCPServerTable" WHERE "server_id" = $1"#;

async fn seed(store: &PostgresServerWrites<'_>, payload: &str) {
    let cipher = cipher(SALT);
    create_mcp_server(store, &cipher, &request(payload), "admin")
        .await
        .unwrap();
}

#[tokio::test]
async fn update_writes_only_the_fields_the_caller_sent() {
    let harness = database().await;
    let store = harness.writes();
    let cipher = cipher(SALT);
    seed(
        &store,
        r#"{"server_id": "srv-1", "server_name": "weather", "alias": "w", "transport": "http",
            "url": "https://a", "description": "old", "allowed_tools": ["t1"],
            "max_concurrent_requests": 7}"#,
    )
    .await;
    let (request, fields) = update_payload(
        r#"{"server_id": "srv-1", "description": "new", "max_concurrent_requests": 30}"#,
    );
    let updated = update_mcp_server(&store, &cipher, &request, "editor", &fields)
        .await
        .unwrap();
    let UpdatedServer::Updated(Some(server)) = updated else {
        panic!("the row exists");
    };
    assert_eq!(server.description.as_deref(), Some("new"));
    assert_eq!(server.max_concurrent_requests, Some(30));
    assert_eq!(server.url.as_deref(), Some("https://a"));
    assert_eq!(server.alias.as_deref(), Some("w"));
    assert_eq!(server.server_name.as_deref(), Some("weather"));
    assert_eq!(server.allowed_tools, ["t1"]);
    assert_eq!(server.transport, MCPTransport::Http);
    assert_eq!(server.created_by.as_deref(), Some("admin"));
    assert_eq!(server.updated_by.as_deref(), Some("editor"));
    harness.close().await;
}

// alias=None clears the prefix, allowed_tools=None is an empty whitelist, a null map clears to {}.
#[tokio::test]
async fn update_clears_alias_and_normalizes_null_json_fields() {
    let harness = database().await;
    let store = harness.writes();
    let cipher = cipher(SALT);
    seed(
        &store,
        r#"{"server_id": "srv-1", "server_name": "weather", "alias": "w",
            "allowed_tools": ["t1"], "tool_name_to_display_name": {"t1": "T1"}}"#,
    )
    .await;
    let (request, fields) = update_payload(
        r#"{"server_id": "srv-1", "alias": null, "allowed_tools": null,
            "tool_name_to_display_name": null}"#,
    );
    let updated = update_mcp_server(&store, &cipher, &request, "editor", &fields)
        .await
        .unwrap();
    let UpdatedServer::Updated(Some(server)) = updated else {
        panic!("the row exists");
    };
    assert_eq!(server.alias, None);
    assert!(server.allowed_tools.is_empty());
    assert_eq!(server.tool_name_to_display_name, Some(BTreeMap::new()));
    assert_eq!(harness.raw_column(ALIAS_SQL, "srv-1").await, None);
    harness.close().await;
}

#[tokio::test]
async fn update_of_a_missing_row_returns_none() {
    let harness = database().await;
    let store = harness.writes();
    let cipher = cipher(SALT);
    let (request, fields) = update_payload(r#"{"server_id": "absent", "description": "x"}"#);
    let updated = update_mcp_server(&store, &cipher, &request, "editor", &fields)
        .await
        .unwrap();
    assert!(matches!(updated, UpdatedServer::Updated(None)));
    assert_eq!(harness.count(SERVER_COUNT_SQL).await, 0);
    harness.close().await;
}

#[tokio::test]
async fn update_without_a_server_id_is_an_error() {
    let harness = database().await;
    let store = harness.writes();
    let cipher = cipher(SALT);
    let (request, fields) = update_payload(r#"{"description": "x"}"#);
    let error = update_mcp_server(&store, &cipher, &request, "editor", &fields)
        .await
        .unwrap_err();
    assert!(matches!(error, ServerWriteError::MissingServerId));
    harness.close().await;
}

#[tokio::test]
async fn update_reports_identifier_conflicts_and_excludes_the_row_itself() {
    let harness = database().await;
    let store = harness.writes();
    let cipher = cipher(SALT);
    seed(
        &store,
        r#"{"server_id": "srv-a", "server_name": "alpha", "alias": "shared"}"#,
    )
    .await;
    seed(&store, r#"{"server_id": "srv-b", "server_name": "beta"}"#).await;
    let (rename, rename_fields) =
        update_payload(r#"{"server_id": "srv-b", "server_name": "ALPHA"}"#);
    let renamed = update_mcp_server(&store, &cipher, &rename, "editor", &rename_fields)
        .await
        .unwrap();
    let UpdatedServer::Conflict(name_conflict) = renamed else {
        panic!("the server_name collides");
    };
    assert_eq!(name_conflict.field, IdentifierField::ServerName);
    assert_eq!(name_conflict.value, "ALPHA");
    assert_eq!(name_conflict.server_id, "srv-a");
    let (realias, realias_fields) = update_payload(r#"{"server_id": "srv-b", "alias": "Shared"}"#);
    let realiased = update_mcp_server(&store, &cipher, &realias, "editor", &realias_fields)
        .await
        .unwrap();
    let UpdatedServer::Conflict(alias_conflict) = realiased else {
        panic!("the alias collides");
    };
    assert_eq!(alias_conflict.field, IdentifierField::Alias);
    let (request, fields) = update_payload(r#"{"server_id": "srv-b", "server_name": "beta"}"#);
    let updated = update_mcp_server(&store, &cipher, &request, "editor", &fields)
        .await
        .unwrap();
    let UpdatedServer::Updated(Some(server)) = updated else {
        panic!("a row keeping its own identifier is no conflict");
    };
    assert_eq!(server.server_name.as_deref(), Some("beta"));
    harness.close().await;
}

// Clearing the alias falls back to the stored server_name, which may belong to another row.
#[tokio::test]
async fn update_clearing_the_alias_locks_the_stored_server_name() {
    let harness = database().await;
    let store = harness.writes();
    let cipher = cipher(SALT);
    seed(&store, r#"{"server_id": "srv-a", "server_name": "alpha"}"#).await;
    seed(
        &store,
        r#"{"server_id": "srv-b", "server_name": "alpha", "alias": "b"}"#,
    )
    .await;
    let (request, fields) = update_payload(r#"{"server_id": "srv-b", "alias": null}"#);
    let blocked = update_mcp_server(&store, &cipher, &request, "editor", &fields)
        .await
        .unwrap();
    let UpdatedServer::Conflict(conflict) = blocked else {
        panic!("the stored server_name collides");
    };
    assert_eq!(conflict.field, IdentifierField::ServerName);
    assert_eq!(conflict.value, "alpha");
    assert_eq!(conflict.server_id, "srv-a");
    let invalidation = RecordingInvalidation::new();
    delete_mcp_server(&store, "srv-a", &invalidation)
        .await
        .unwrap();
    let updated = update_mcp_server(&store, &cipher, &request, "editor", &fields)
        .await
        .unwrap();
    let UpdatedServer::Updated(Some(server)) = updated else {
        panic!("the collision is gone");
    };
    assert_eq!(server.alias, None);
    assert!(harness.raw_column(EXISTS_SQL, "srv-b").await.is_some());
    harness.close().await;
}
