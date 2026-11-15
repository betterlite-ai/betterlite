use crate::mcp::table_access::filter::ServerFilter;
use crate::mcp::table_access::{get_active_submitted_mcp_server_ids_for_user, get_mcp_submissions};

use super::support::{FakeStore, cipher, row};

#[tokio::test]
async fn get_mcp_submissions_cap_at_500_newest_first_with_counts() {
    let store = FakeStore::new(vec![
        row(
            r#"{"server_id":"old","submitted_at":"2026-01-01T00:00:00Z","approval_status":"rejected","transport":"http"}"#,
        ),
        row(
            r#"{"server_id":"new","submitted_at":"2026-06-01T00:00:00Z","approval_status":"pending_review","transport":"http"}"#,
        ),
        row(
            r#"{"server_id":"middle","submitted_at":"2026-03-01T00:00:00Z","approval_status":"active","transport":"http"}"#,
        ),
        row(r#"{"server_id":"unsubmitted","approval_status":"active","transport":"http"}"#),
    ]);
    let summary = get_mcp_submissions(&store, &cipher())
        .await
        .expect("the fake store never fails");
    assert_eq!(summary.total, 3);
    assert_eq!(summary.pending_review, 1);
    assert_eq!(summary.active, 1);
    assert_eq!(summary.rejected, 1);
    let ids: Vec<&str> = summary
        .items
        .iter()
        .map(|server| server.server_id.as_str())
        .collect();
    assert_eq!(ids, vec!["new", "middle", "old"]);
    let finds = store.finds();
    assert!(matches!(
        finds[0].filter.as_ref(),
        Some(ServerFilter::Submitted)
    ));
    assert!(finds[0].descending);
    assert_eq!(finds[0].take, Some(500));
}

#[tokio::test]
async fn active_submissions_of_a_user_are_their_active_servers_only() {
    let store = FakeStore::new(vec![
        row(
            r#"{"server_id":"mine-active","submitted_by":"u1","approval_status":"active","transport":"http"}"#,
        ),
        row(
            r#"{"server_id":"mine-pending","submitted_by":"u1","approval_status":"pending_review","transport":"http"}"#,
        ),
        row(
            r#"{"server_id":"other-active","submitted_by":"u2","approval_status":"active","transport":"http"}"#,
        ),
    ]);
    let ids = get_active_submitted_mcp_server_ids_for_user(&store, "u1")
        .await
        .expect("the fake store never fails");
    assert_eq!(ids, vec!["mine-active"]);
    let finds = store.finds();
    assert!(matches!(
        finds[0].filter.as_ref(),
        Some(ServerFilter::SubmittedByActive { user_id }) if user_id.as_str() == "u1"
    ));
}

#[tokio::test]
async fn active_submissions_are_empty_for_an_empty_user() {
    let store = FakeStore::new(vec![row(
        r#"{"server_id":"someone","submitted_by":"u1","approval_status":"active","transport":"http"}"#,
    )]);
    let ids = get_active_submitted_mcp_server_ids_for_user(&store, "")
        .await
        .expect("an empty user needs no query");
    assert!(ids.is_empty());
    assert!(store.finds().is_empty());
}
