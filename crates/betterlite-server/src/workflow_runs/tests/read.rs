use std::sync::{Arc, Mutex};

use axum::http::{Method, StatusCode};
use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use rstest::rstest;

use super::support::{
    DetailReply, FakeStore, call, master, member, principal, proxy_admin, viewer,
};
use crate::workflow_runs::models::WorkflowRunRow;

fn seeded() -> Arc<FakeStore> {
    let store = Arc::new(FakeStore::default());
    let owned = store.seed_run(Some("hash-1"), "review", "running");
    store.seed_event(&owned, 0);
    store.seed_event(&owned, 2);
    store.seed_event(&owned, 1);
    store.seed_run(Some("hash-2"), "review", "paused");
    store.seed_run(None, "review", "pending");
    store
}

#[tokio::test]
async fn get_returns_the_run_with_only_its_latest_event() {
    let store = seeded();
    let reply = call(
        Some(&store),
        Some(&member("hash-1")),
        Method::GET,
        "/v1/workflows/runs/run-0",
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let row: WorkflowRunRow = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(row.run_id, "run-0");
    let events = row.events.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events.first().unwrap().sequence_number, 2_i32);
}

#[tokio::test]
async fn get_for_a_run_without_events_returns_an_empty_event_list() {
    let store = seeded();
    let reply = call(
        Some(&store),
        Some(&member("hash-2")),
        Method::GET,
        "/v1/workflows/runs/run-1",
        "",
    )
    .await;
    let row: WorkflowRunRow = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(row.events, Some(Vec::new()));
}

#[rstest]
#[case::owner(member("hash-1"), "run-0", StatusCode::OK)]
#[case::other_member(member("hash-2"), "run-0", StatusCode::NOT_FOUND)]
#[case::member_on_a_master_run(member("hash-1"), "run-2", StatusCode::NOT_FOUND)]
#[case::member_without_a_key(principal("virtual-key", vec![Role::TeamMember], None), "run-2", StatusCode::NOT_FOUND)]
#[case::view_only_reads_foreign(viewer("hash-9"), "run-0", StatusCode::OK)]
#[case::proxy_admin_reads_foreign(proxy_admin("hash-9"), "run-1", StatusCode::OK)]
#[case::master_reads_foreign(master(), "run-0", StatusCode::OK)]
#[case::master_reads_its_own(master(), "run-2", StatusCode::OK)]
#[case::missing_for_a_member(member("hash-1"), "run-404", StatusCode::NOT_FOUND)]
#[case::missing_for_the_master(master(), "run-404", StatusCode::NOT_FOUND)]
#[tokio::test]
async fn get_applies_the_admin_view_check(
    #[case] caller: Principal,
    #[case] run_id: &str,
    #[case] status: StatusCode,
) {
    let store = seeded();
    let uri = format!("/v1/workflows/runs/{run_id}");
    let reply = call(Some(&store), Some(&caller), Method::GET, &uri, "").await;
    assert_eq!(reply.status, status);
    if status == StatusCode::NOT_FOUND {
        let detail: DetailReply = serde_json::from_str(&reply.body).unwrap();
        assert_eq!(detail.detail, format!("Run '{run_id}' not found"));
    }
}

#[tokio::test]
async fn get_without_a_database_is_a_500_with_the_db_error() {
    let reply = call(
        None,
        Some(&master()),
        Method::GET,
        "/v1/workflows/runs/run-0",
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let detail: DetailReply = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(
        detail.detail,
        CommonProxyErrors::DbNotConnectedError.as_str()
    );
}

#[tokio::test]
async fn get_store_failure_is_a_500() {
    let store = Arc::new(FakeStore {
        state: Mutex::default(),
        failing: true,
        vanish_on_update: false,
    });
    let reply = call(
        Some(&store),
        Some(&master()),
        Method::GET,
        "/v1/workflows/runs/run-0",
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn get_without_a_caller_is_unauthenticated() {
    let store = seeded();
    let reply = call(
        Some(&store),
        None,
        Method::GET,
        "/v1/workflows/runs/run-0",
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
}
