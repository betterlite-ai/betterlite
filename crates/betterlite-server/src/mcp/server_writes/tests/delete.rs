use chrono::Utc;

use super::support::cipher::{SALT, cipher};
use super::support::database::{
    CHILD_COUNT_SQL, ENV_VAR_COUNT_SQL, OAUTH_COUNT_SQL, SERVER_COUNT_SQL, database,
};
use super::support::fake::{FakeStore, Fault, RecordingInvalidation, active_row};
use super::support::logs::capture_logs;
use super::support::request;
use crate::mcp::server_writes::DeletedServer;
use crate::mcp::server_writes::create::create_mcp_server;
use crate::mcp::server_writes::delete::delete_mcp_server;
use crate::mcp::server_writes::store::PostgresServerWrites;

async fn seed_server(store: &PostgresServerWrites<'_>, payload: &str) {
    create_mcp_server(store, &cipher(SALT), &request(payload), "admin")
        .await
        .unwrap();
}

#[tokio::test]
async fn delete_removes_the_row_children_and_invalidates_each_user() {
    let harness = database().await;
    let store = harness.writes();
    seed_server(
        &store,
        r#"{"server_id": "srv-1", "server_name": "weather", "transport": "http"}"#,
    )
    .await;
    harness.seed_children("srv-1", &["u1", "u2"]).await;
    let invalidation = RecordingInvalidation::new();
    let deleted = delete_mcp_server(&store, "srv-1", &invalidation)
        .await
        .unwrap();
    assert_eq!(
        deleted,
        Some(DeletedServer {
            server_id: "srv-1".to_owned(),
            server_name: Some("weather".to_owned()),
        })
    );
    assert_eq!(harness.count(SERVER_COUNT_SQL).await, 0);
    assert_eq!(harness.count(CHILD_COUNT_SQL).await, 0);
    assert_eq!(harness.count(ENV_VAR_COUNT_SQL).await, 0);
    assert_eq!(harness.count(OAUTH_COUNT_SQL).await, 0);
    let mut calls = invalidation.calls();
    calls.sort();
    assert_eq!(
        calls,
        [
            ("u1".to_owned(), "srv-1".to_owned()),
            ("u2".to_owned(), "srv-1".to_owned()),
        ]
    );
    harness.close().await;
}

#[tokio::test]
async fn a_second_delete_returns_none_without_invalidating() {
    let harness = database().await;
    let store = harness.writes();
    seed_server(
        &store,
        r#"{"server_id": "srv-1", "server_name": "weather", "transport": "http"}"#,
    )
    .await;
    let invalidation = RecordingInvalidation::new();
    delete_mcp_server(&store, "srv-1", &invalidation)
        .await
        .unwrap();
    assert!(
        delete_mcp_server(&store, "srv-1", &invalidation)
            .await
            .unwrap()
            .is_none()
    );
    assert!(invalidation.calls().is_empty());
    harness.close().await;
}

// A cache that rejects one user must not abort the delete or the invalidation of the others.
#[tokio::test]
async fn delete_logs_a_failed_invalidation_and_still_invalidates_the_next_user() {
    let harness = database().await;
    let store = harness.writes();
    seed_server(
        &store,
        r#"{"server_id": "srv-1", "server_name": "weather", "transport": "http"}"#,
    )
    .await;
    harness.seed_children("srv-1", &["u1", "u2"]).await;
    let invalidation = RecordingInvalidation::new();
    invalidation.fail_user("u1");
    let (logs, guard) = capture_logs();
    let deleted = delete_mcp_server(&store, "srv-1", &invalidation)
        .await
        .unwrap();
    drop(guard);
    assert!(deleted.is_some());
    assert_eq!(harness.count(SERVER_COUNT_SQL).await, 0);
    assert_eq!(
        invalidation.calls(),
        [("u2".to_owned(), "srv-1".to_owned())]
    );
    assert!(logs.contents().contains("could not be invalidated"));
    harness.close().await;
}

#[tokio::test]
async fn delete_warns_when_credential_enumeration_fails_and_still_cleans_up() {
    let mut store = FakeStore::new(vec![active_row("srv-1", &Utc::now().to_rfc3339())]);
    store.fault = Fault::CredentialEnumeration;
    let invalidation = RecordingInvalidation::new();
    let (logs, guard) = capture_logs();
    let deleted = delete_mcp_server(&store, "srv-1", &invalidation)
        .await
        .unwrap();
    drop(guard);
    assert!(deleted.is_some());
    assert!(store.rows().is_empty());
    assert_eq!(store.child_deletes().len(), 3);
    assert!(invalidation.calls().is_empty());
    assert!(logs.contents().contains("credential enumeration failed"));
}

#[tokio::test]
async fn delete_warns_on_a_failed_child_cleanup_and_still_invalidates() {
    let mut store = FakeStore::new(vec![active_row("srv-1", &Utc::now().to_rfc3339())])
        .with_credential_users("srv-1", &["u1"]);
    store.fault = Fault::ChildCleanup;
    let invalidation = RecordingInvalidation::new();
    let (logs, guard) = capture_logs();
    let deleted = delete_mcp_server(&store, "srv-1", &invalidation)
        .await
        .unwrap();
    drop(guard);
    assert!(deleted.is_some());
    assert!(store.child_deletes().is_empty());
    assert_eq!(
        invalidation.calls(),
        [("u1".to_owned(), "srv-1".to_owned())]
    );
    let lines = logs.contents();
    assert!(lines.contains("per-user credential cleanup failed"));
    assert!(lines.contains("per-user env var cleanup failed"));
    assert!(lines.contains("per-user OAuth client cleanup failed"));
}

#[tokio::test]
async fn delete_logs_each_failed_invalidation_and_continues() {
    let store = FakeStore::new(vec![active_row("srv-1", &Utc::now().to_rfc3339())])
        .with_credential_users("srv-1", &["u1", "u2"]);
    let invalidation = RecordingInvalidation::new();
    invalidation.fail_user("u1");
    let (logs, guard) = capture_logs();
    delete_mcp_server(&store, "srv-1", &invalidation)
        .await
        .unwrap();
    drop(guard);
    assert_eq!(
        invalidation.calls(),
        [("u2".to_owned(), "srv-1".to_owned())]
    );
    assert!(logs.contents().contains("could not be invalidated"));
}
