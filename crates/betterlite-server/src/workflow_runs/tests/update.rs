use std::sync::{Arc, Mutex};

use axum::http::{Method, StatusCode};
use betterlite_auth::auth_identity::records::Principal;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use rstest::rstest;

use super::support::{
    DetailReply, FakeStore, LocReply, ValidationReply, call, master, member, proxy_admin, viewer,
};
use crate::workflow_runs::models::{RunUpdateData, WorkflowRunRow, WorkflowRunStatus};

fn seeded() -> Arc<FakeStore> {
    let store = Arc::new(FakeStore::default());
    store.seed_run(Some("hash-1"), "review", "running");
    store.seed_run(Some("hash-2"), "review", "paused");
    store
}

#[tokio::test]
async fn patch_updates_the_owners_run_with_json_strings() {
    let store = seeded();
    let body = r#"{"status":"completed","output":{"ok":true},"metadata":{"k":"v"}}"#;
    let reply = call(
        Some(&store),
        Some(&member("hash-1")),
        Method::PATCH,
        "/v1/workflows/runs/run-0",
        body,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let row: WorkflowRunRow = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(row.status, "completed");
    assert_eq!(
        store.recorded().updates,
        vec![(
            "run-0".to_owned(),
            RunUpdateData {
                status: Some(WorkflowRunStatus::Completed),
                output: Some(r#"{"ok":true}"#.to_owned()),
                metadata: Some(r#"{"k":"v"}"#.to_owned()),
            }
        )]
    );
}

#[rstest]
#[case::pending("pending", WorkflowRunStatus::Pending)]
#[case::running("running", WorkflowRunStatus::Running)]
#[case::paused("paused", WorkflowRunStatus::Paused)]
#[case::completed("completed", WorkflowRunStatus::Completed)]
#[case::failed("failed", WorkflowRunStatus::Failed)]
#[tokio::test]
async fn patch_accepts_each_declared_status(#[case] text: &str, #[case] status: WorkflowRunStatus) {
    let store = seeded();
    let body = format!(r#"{{"status":"{text}"}}"#);
    let reply = call(
        Some(&store),
        Some(&member("hash-1")),
        Method::PATCH,
        "/v1/workflows/runs/run-0",
        &body,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let update = store.recorded().updates.first().unwrap().1.clone();
    assert_eq!(update.status, Some(status));
    assert_eq!(update.output, None);
    assert_eq!(update.metadata, None);
}

#[rstest]
#[case::empty_object("{}")]
#[case::null_status(r#"{"status":null}"#)]
#[case::all_null(r#"{"status":null,"output":null,"metadata":null}"#)]
#[tokio::test]
async fn patch_with_nothing_to_update_is_a_400_even_for_a_missing_run(#[case] body: &str) {
    let store = seeded();
    let reply = call(
        Some(&store),
        Some(&member("hash-1")),
        Method::PATCH,
        "/v1/workflows/runs/run-404",
        body,
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    let detail: DetailReply = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(detail.detail, "No fields to update");
    assert!(store.recorded().updates.is_empty());
}

#[rstest]
#[case::owner(member("hash-1"), "run-0", StatusCode::OK)]
#[case::foreign_member(member("hash-2"), "run-0", StatusCode::NOT_FOUND)]
#[case::view_only_on_foreign(viewer("hash-9"), "run-0", StatusCode::NOT_FOUND)]
#[case::view_only_on_own(viewer("hash-1"), "run-0", StatusCode::OK)]
#[case::proxy_admin_on_foreign(proxy_admin("hash-9"), "run-0", StatusCode::OK)]
#[case::master_on_foreign(master(), "run-1", StatusCode::OK)]
#[case::missing_for_a_member(member("hash-1"), "run-404", StatusCode::NOT_FOUND)]
#[case::missing_for_the_master(master(), "run-404", StatusCode::NOT_FOUND)]
#[tokio::test]
async fn patch_enforces_ownership_except_for_a_full_proxy_admin(
    #[case] caller: Principal,
    #[case] run_id: &str,
    #[case] status: StatusCode,
) {
    let store = seeded();
    let uri = format!("/v1/workflows/runs/{run_id}");
    let reply = call(
        Some(&store),
        Some(&caller),
        Method::PATCH,
        &uri,
        r#"{"status":"paused"}"#,
    )
    .await;
    assert_eq!(reply.status, status);
    assert_eq!(
        store.recorded().updates.len(),
        usize::from(status == StatusCode::OK)
    );
    if status == StatusCode::NOT_FOUND {
        let detail: DetailReply = serde_json::from_str(&reply.body).unwrap();
        assert_eq!(detail.detail, format!("Run '{run_id}' not found"));
    }
}

#[tokio::test]
async fn patch_is_a_404_when_the_run_disappears_before_the_write() {
    let store = Arc::new(FakeStore {
        state: Mutex::default(),
        failing: false,
        vanish_on_update: true,
    });
    store.seed_run(Some("hash-1"), "review", "running");
    let reply = call(
        Some(&store),
        Some(&member("hash-1")),
        Method::PATCH,
        "/v1/workflows/runs/run-0",
        r#"{"status":"failed"}"#,
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    let detail: DetailReply = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(detail.detail, "Run 'run-0' not found");
}

#[tokio::test]
async fn patch_rejects_a_status_outside_the_literal() {
    let store = seeded();
    let reply = call(
        Some(&store),
        Some(&member("hash-1")),
        Method::PATCH,
        "/v1/workflows/runs/run-0",
        r#"{"status":"done"}"#,
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let parsed: ValidationReply = serde_json::from_str(&reply.body).unwrap();
    let issue = parsed.detail.first().unwrap();
    assert_eq!(issue.issue_type, "value_error");
    assert_eq!(
        issue.loc,
        [
            LocReply::Name("body".to_owned()),
            LocReply::Name("status".to_owned())
        ]
    );
    assert!(store.recorded().updates.is_empty());
}

#[rstest]
#[case::empty_body("{}")]
#[case::with_status(r#"{"status":"running"}"#)]
#[tokio::test]
async fn patch_without_a_database_is_a_500_with_the_db_error(#[case] body: &str) {
    let reply = call(
        None,
        Some(&master()),
        Method::PATCH,
        "/v1/workflows/runs/run-0",
        body,
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
async fn patch_store_failure_is_a_500() {
    let store = Arc::new(FakeStore {
        state: Mutex::default(),
        failing: true,
        vanish_on_update: false,
    });
    let reply = call(
        Some(&store),
        Some(&master()),
        Method::PATCH,
        "/v1/workflows/runs/run-0",
        r#"{"status":"running"}"#,
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn patch_without_a_caller_is_unauthenticated() {
    let store = seeded();
    let reply = call(
        Some(&store),
        None,
        Method::PATCH,
        "/v1/workflows/runs/run-0",
        r#"{"status":"running"}"#,
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert!(store.recorded().updates.is_empty());
}
