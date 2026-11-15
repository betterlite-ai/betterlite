use std::collections::BTreeMap;

use chrono::DateTime;

use crate::mcp::table_access::rows::{RowFields, ServerUpdate};
use crate::mcp::table_access::types::McpTableError;
use crate::mcp::table_access::{
    _db_update_mcp_server_row, approve_mcp_server, reject_mcp_server, set_mcp_server_pinned_tools,
};
use crate::mcp_stored_contracts::{JsonValue, PinnedMCPTool};

use super::support::{FakeStore, cipher, row};

fn only_update(store: &FakeStore) -> ServerUpdate {
    let updates = store.updates();
    assert_eq!(updates.len(), 1);
    updates.first().cloned().expect("one update ran")
}

#[tokio::test]
async fn update_of_a_missing_server_is_not_found() {
    let store = FakeStore::new(Vec::new());
    let error = _db_update_mcp_server_row(
        &store,
        "missing",
        RowFields::texts(&[("updated_by", "admin")]),
    )
    .await
    .expect_err("the fake store has no such row");
    assert!(matches!(
        &error,
        McpTableError::ServerNotFound { server_id } if server_id.as_str() == "missing"
    ));
    assert_eq!(
        error.to_string(),
        "MCP server not found, passed server_id=missing"
    );
}

#[tokio::test]
async fn update_of_a_stored_server_returns_the_written_row() {
    let store = FakeStore::new(vec![row(r#"{"server_id":"s1","transport":"http"}"#)]);
    let written =
        _db_update_mcp_server_row(&store, "s1", RowFields::texts(&[("updated_by", "admin")]))
            .await
            .expect("s1 is stored");
    assert_eq!(
        written,
        row(r#"{"server_id":"s1","transport":"http","updated_by":"admin"}"#)
    );
    assert_eq!(only_update(&store).server_id, "s1");
}

#[tokio::test]
async fn approve_marks_the_server_active_with_a_review_time() {
    let store = FakeStore::new(vec![row(
        r#"{"server_id":"s1","approval_status":"pending_review","transport":"http"}"#,
    )]);
    let server = approve_mcp_server(&store, &cipher(), "s1", "admin")
        .await
        .expect("the fake store never fails");
    assert_eq!(server.approval_status.as_deref(), Some("active"));
    assert_eq!(server.updated_by.as_deref(), Some("admin"));
    assert!(server.reviewed_at.is_some());
    let update = only_update(&store);
    assert_eq!(
        update.data.0.keys().copied().collect::<Vec<_>>(),
        vec!["approval_status", "reviewed_at", "updated_by"]
    );
    let Some(JsonValue::String(reviewed_at)) = update.data.0.get("reviewed_at") else {
        panic!("reviewed_at is stored as text");
    };
    let parsed = DateTime::parse_from_rfc3339(reviewed_at).expect("reviewed_at is RFC 3339");
    assert_eq!(parsed.offset().local_minus_utc(), 0);
}

#[tokio::test]
async fn approve_of_a_missing_server_is_not_found() {
    let store = FakeStore::new(Vec::new());
    let error = approve_mcp_server(&store, &cipher(), "gone", "admin")
        .await
        .expect_err("the fake store has no such row");
    assert!(matches!(
        error,
        McpTableError::ServerNotFound { server_id } if server_id.as_str() == "gone"
    ));
}

#[tokio::test]
async fn reject_records_rejected_status_and_the_notes_when_given() {
    let store = FakeStore::new(vec![row(
        r#"{"server_id":"s1","approval_status":"pending_review","review_notes":"old","transport":"http"}"#,
    )]);
    let server = reject_mcp_server(&store, &cipher(), "s1", "admin", Some("unsafe tools"))
        .await
        .expect("the fake store never fails");
    assert_eq!(server.approval_status.as_deref(), Some("rejected"));
    assert_eq!(server.review_notes.as_deref(), Some("unsafe tools"));
    assert_eq!(server.updated_by.as_deref(), Some("admin"));
    assert!(server.reviewed_at.is_some());
}

#[tokio::test]
async fn reject_without_notes_leaves_the_stored_notes_alone() {
    let store = FakeStore::new(vec![row(
        r#"{"server_id":"s1","approval_status":"pending_review","review_notes":"old","transport":"http"}"#,
    )]);
    let server = reject_mcp_server(&store, &cipher(), "s1", "admin", None)
        .await
        .expect("the fake store never fails");
    assert_eq!(server.review_notes.as_deref(), Some("old"));
    assert_eq!(
        only_update(&store)
            .data
            .0
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        vec!["approval_status", "reviewed_at", "updated_by"]
    );
}

#[tokio::test]
async fn pinned_tools_are_stored_as_a_json_snapshot() {
    let store = FakeStore::new(vec![row(r#"{"server_id":"s1","transport":"http"}"#)]);
    let tools = serde_json::from_str::<BTreeMap<String, PinnedMCPTool>>(
        r#"{"echo":{"description":"Echo","input_schema":{"type":"object"}}}"#,
    )
    .expect("the pinned tools are valid");
    let server = set_mcp_server_pinned_tools(&store, &cipher(), "s1", Some(&tools), "admin")
        .await
        .expect("the fake store never fails")
        .expect("the server exists");
    let pinned = server.pinned_tools.as_ref().expect("the pin is set");
    assert_eq!(
        pinned
            .get("echo")
            .expect("echo is pinned")
            .as_ref()
            .description,
        "Echo"
    );
    assert_eq!(server.updated_by.as_deref(), Some("admin"));
    assert_eq!(
        only_update(&store).data.0.get("pinned_tools"),
        Some(&JsonValue::String(
            r#"{"echo":{"description":"Echo","input_schema":{"type":"object"}}}"#.to_owned()
        ))
    );
}

#[tokio::test]
async fn unpinning_stores_an_empty_snapshot() {
    let store = FakeStore::new(vec![row(r#"{"server_id":"s1","transport":"http"}"#)]);
    let server = set_mcp_server_pinned_tools(&store, &cipher(), "s1", None, "admin")
        .await
        .expect("the fake store never fails")
        .expect("the server exists");
    assert!(server.pinned_tools.is_none());
    assert_eq!(
        only_update(&store).data.0.get("pinned_tools"),
        Some(&JsonValue::String("{}".to_owned()))
    );
}

#[tokio::test]
async fn pinned_tools_of_an_unknown_server_are_not_written() {
    let store = FakeStore::new(Vec::new());
    let server = set_mcp_server_pinned_tools(&store, &cipher(), "gone", None, "admin")
        .await
        .expect("the fake store never fails");
    assert!(server.is_none());
    assert!(store.updates().is_empty());
}
