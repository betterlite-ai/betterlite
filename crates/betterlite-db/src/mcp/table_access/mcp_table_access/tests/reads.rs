use crate::managed_resource_rows::ManagedRowsDecodeError;
use crate::managed_resource_rows::mcp::McpServerTable;
use crate::mcp::table_access::filter::{FindServers, ServerFilter};
use crate::mcp::table_access::server_reads::{
    _db_find_mcp_server_rows, get_mcp_server, get_mcp_servers,
};
use crate::mcp::table_access::types::McpTableError;
use crate::mcp::table_access::{
    _db_find_mcp_server_row, _readable_mcp_servers, get_all_mcp_servers,
};

use super::support::{FakeStore, cipher, row};

fn first_find(store: &FakeStore) -> FindServers {
    store
        .finds()
        .first()
        .cloned()
        .expect("the read queried the table")
}

#[tokio::test]
async fn get_all_servers_skip_draft_rows() {
    let store = FakeStore::new(vec![
        row(r#"{"server_id":"active","approval_status":"active","transport":"http"}"#),
        row(r#"{"server_id":"draft","approval_status":"draft","transport":"http"}"#),
    ]);
    let servers = get_all_mcp_servers(&store, &cipher(), None)
        .await
        .expect("the fake store never fails");
    assert_eq!(server_ids(&servers), vec!["active"]);
    let find = first_find(&store);
    assert!(matches!(find.filter, Some(ServerFilter::NotDraft)));
    assert!(!find.descending);
    assert_eq!(find.take, None);
}

#[tokio::test]
async fn get_all_servers_keep_null_approval_status_rows() {
    let store = FakeStore::new(vec![
        row(r#"{"server_id":"legacy","transport":"http"}"#),
        row(r#"{"server_id":"null","approval_status":null,"transport":"http"}"#),
        row(r#"{"server_id":"draft","approval_status":"draft","transport":"http"}"#),
    ]);
    let servers = get_all_mcp_servers(&store, &cipher(), None)
        .await
        .expect("the fake store never fails");
    assert_eq!(server_ids(&servers), vec!["legacy", "null"]);
}

#[tokio::test]
async fn get_all_servers_filter_by_the_requested_status() {
    let store = FakeStore::new(vec![
        row(r#"{"server_id":"live","approval_status":"active","transport":"http"}"#),
        row(r#"{"server_id":"denied","approval_status":"rejected","transport":"http"}"#),
    ]);
    let servers = get_all_mcp_servers(&store, &cipher(), Some("rejected"))
        .await
        .expect("the fake store never fails");
    assert_eq!(server_ids(&servers), vec!["denied"]);
    assert!(matches!(
        first_find(&store).filter,
        Some(ServerFilter::ApprovalStatus(status)) if status.as_str() == "rejected"
    ));
}

#[tokio::test]
async fn server_rows_are_read_with_the_given_where_or_unfiltered() {
    let store = FakeStore::new(vec![
        row(r#"{"server_id":"s1","approval_status":"draft","transport":"http"}"#),
        row(r#"{"server_id":"s2","transport":"http"}"#),
    ]);
    let every = _db_find_mcp_server_rows(&store, None)
        .await
        .expect("the fake store never fails");
    assert_eq!(every.len(), 2);
    let kept = _db_find_mcp_server_rows(&store, Some(ServerFilter::NotDraft))
        .await
        .expect("the fake store never fails");
    assert_eq!(kept, vec![row(r#"{"server_id":"s2","transport":"http"}"#)]);
    let find = first_find(&store);
    assert!(find.filter.is_none());
    assert_eq!(find.take, None);
}

#[tokio::test]
async fn the_server_row_is_found_by_id_or_none() {
    let store = FakeStore::new(vec![row(r#"{"server_id":"s1","transport":"http"}"#)]);
    let found = _db_find_mcp_server_row(&store, "s1")
        .await
        .expect("the fake store never fails");
    assert_eq!(found, Some(row(r#"{"server_id":"s1","transport":"http"}"#)));
    let missing = _db_find_mcp_server_row(&store, "s2")
        .await
        .expect("the fake store never fails");
    assert_eq!(missing, None);
    assert_eq!(first_find(&store).take, Some(1));
}

#[tokio::test]
async fn get_server_is_none_for_an_unknown_id() {
    let store = FakeStore::new(Vec::new());
    let server = get_mcp_server(&store, &cipher(), "missing")
        .await
        .expect("the fake store never fails");
    assert!(server.is_none());
}

#[tokio::test]
async fn get_server_returns_the_decoded_row() {
    let store = FakeStore::new(vec![row(
        r#"{"server_id":"s1","approval_status":"active","transport":"http"}"#,
    )]);
    let server = get_mcp_server(&store, &cipher(), "s1")
        .await
        .expect("the fake store never fails")
        .expect("s1 is stored");
    assert_eq!(server.server_id, "s1");
    assert_eq!(server.approval_status.as_deref(), Some("active"));
}

#[tokio::test]
async fn get_servers_read_the_requested_ids_in_table_order() {
    let store = FakeStore::new(vec![
        row(r#"{"server_id":"s1","transport":"http"}"#),
        row(r#"{"server_id":"s2","transport":"http"}"#),
        row(r#"{"server_id":"s3","transport":"http"}"#),
    ]);
    let requested = vec!["s3".to_owned(), "s1".to_owned()];
    let servers = get_mcp_servers(&store, &cipher(), &requested)
        .await
        .expect("the fake store never fails");
    assert_eq!(server_ids(&servers), vec!["s1", "s3"]);
    assert!(matches!(
        first_find(&store).filter,
        Some(ServerFilter::ServerIds(ids)) if ids == requested
    ));
}

#[test]
fn readable_servers_skip_rows_whose_secret_map_fails_to_decrypt() {
    let rows = vec![
        row(r#"{"server_id":"good","transport":"http"}"#),
        row(r#"{"server_id":"bad","env":"not-a-secret","transport":"http"}"#),
    ];
    let servers = _readable_mcp_servers(rows, &cipher()).expect("only the bad row is skipped");
    assert_eq!(server_ids(&servers), vec!["good"]);
}

#[test]
fn readable_servers_report_other_decode_failures() {
    let rows = vec![row(r#"{"server_id":"broken"}"#)];
    let error = _readable_mcp_servers(rows, &cipher()).expect_err("a row without transport fails");
    assert!(matches!(error, McpTableError::Decode(_)));
}

#[tokio::test]
async fn get_server_reports_an_undecryptable_secret_map() {
    let store = FakeStore::new(vec![row(
        r#"{"server_id":"bad","env":"not-a-secret","transport":"http"}"#,
    )]);
    let error = get_mcp_server(&store, &cipher(), "bad")
        .await
        .expect_err("the bad row is reported instead of skipped");
    assert!(matches!(
        error,
        McpTableError::Decode(ManagedRowsDecodeError::SecretMap(_))
    ));
}

fn server_ids(servers: &[McpServerTable]) -> Vec<&str> {
    servers
        .iter()
        .map(|server| server.server_id.as_str())
        .collect()
}
