use crate::managed_resource_rows::ManagedRowsDecodeError;
use crate::managed_resource_rows::mcp::McpServerTable;
use crate::mcp::table_access::filter::ServerFilter;
use crate::mcp::table_access::server_reads::{get_mcp_server, get_mcp_servers};
use crate::mcp::table_access::types::McpTableError;
use crate::mcp::table_access::{_readable_mcp_servers, get_all_mcp_servers};

use super::support::{FakeStore, cipher, row};

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
    let finds = store.finds();
    assert!(matches!(
        finds[0].filter.as_ref(),
        Some(ServerFilter::NotDraft)
    ));
    assert!(!finds[0].descending);
    assert_eq!(finds[0].take, None);
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
    let finds = store.finds();
    assert!(matches!(
        finds[0].filter.as_ref(),
        Some(ServerFilter::ApprovalStatus(status)) if status.as_str() == "rejected"
    ));
}

#[tokio::test]
async fn get_server_is_none_for_an_unknown_id() {
    let store = FakeStore::new(Vec::new());
    let server = get_mcp_server(&store, &cipher(), "missing")
        .await
        .expect("the fake store never fails");
    assert!(server.is_none());
    assert_eq!(store.finds()[0].take, Some(1));
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
    let finds = store.finds();
    assert!(matches!(
        finds[0].filter.as_ref(),
        Some(ServerFilter::ServerIds(ids)) if ids.as_slice() == requested.as_slice()
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

#[tokio::test]
async fn get_server_reports_an_undecryptable_secret_map() {
    let store = FakeStore::new(vec![row(
        r#"{"server_id":"bad","env":"not-a-secret","transport":"http"}"#,
    )]);
    let error = get_mcp_server(&store, &cipher(), "bad")
        .await
        .err()
        .expect("the bad row is reported instead of skipped");
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
