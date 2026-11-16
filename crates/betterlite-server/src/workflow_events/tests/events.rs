use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use rstest::rstest;

use super::support::{
    Attempt, DetailReply, FailureKind, FakeStore, LocReply, ValidationReply, master, member, post,
    proxy_admin, viewer,
};
use crate::workflow_events::error::CONFLICT_DETAIL;
use crate::workflow_events::models::EventCreateData;
use crate::workflow_runs::models::{SequenceTable, WorkflowEventRow, WorkflowRunStatus};

fn event_uri(run_id: &str) -> String {
    format!("/v1/workflows/runs/{run_id}/events")
}

fn an_event(event_type: &str) -> String {
    format!(r#"{{"event_type":"{event_type}","step_name":"build"}}"#)
}

#[tokio::test]
async fn an_event_appends_after_the_highest_sequence_of_its_run() {
    let store = Arc::new(FakeStore::default());
    let run_id = store.seed_run(Some("hash-1"));
    store.seed_event(&run_id, 0);
    store.seed_event(&run_id, 4);
    let reply = post(
        Some(&store),
        Some(&member("hash-1")),
        &event_uri(&run_id),
        &an_event("hook.waiting"),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let row: WorkflowEventRow = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(row.sequence_number, 5);
    assert_eq!(row.run_id, run_id);
    assert_eq!(row.event_type, "hook.waiting");
    assert_eq!(row.step_name, "build");
    assert_eq!(row.event_id, format!("event-{run_id}-5"));
    assert_eq!(row.data, None);
    assert_eq!(row.run, None);
}

#[tokio::test]
async fn an_event_on_an_empty_run_starts_at_zero() {
    let store = Arc::new(FakeStore::default());
    let run_id = store.seed_run(Some("hash-1"));
    let other = store.seed_run(Some("hash-1"));
    store.seed_event(&other, 7);
    let reply = post(
        Some(&store),
        Some(&member("hash-1")),
        &event_uri(&run_id),
        &an_event("step.started"),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(store.event_writes().first().unwrap().sequence_number, 0);
}

#[tokio::test]
async fn an_event_stores_its_data_as_json() {
    let store = Arc::new(FakeStore::default());
    let run_id = store.seed_run(Some("hash-1"));
    let body =
        r#"{"event_type":"step.started","step_name":"build","data":{"pr":7,"nested":{"ok":true}}}"#;
    let reply = post(
        Some(&store),
        Some(&member("hash-1")),
        &event_uri(&run_id),
        body,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        store.event_writes(),
        vec![EventCreateData {
            run_id: run_id.clone(),
            event_type: "step.started".to_owned(),
            step_name: "build".to_owned(),
            sequence_number: 0,
            data: Some(r#"{"nested":{"ok":true},"pr":7}"#.to_owned()),
        }]
    );
    let row: WorkflowEventRow = serde_json::from_str(&reply.body).unwrap();
    let expected: JsonData = serde_json::from_str(r#"{"nested":{"ok":true},"pr":7}"#).unwrap();
    assert_eq!(row.data, Some(expected));
}

#[tokio::test]
async fn an_event_without_data_stores_none() {
    let store = Arc::new(FakeStore::default());
    let run_id = store.seed_run(Some("hash-1"));
    let reply = post(
        Some(&store),
        Some(&member("hash-1")),
        &event_uri(&run_id),
        &an_event("step.started"),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(store.event_writes().first().unwrap().data, None);
}

#[rstest]
#[case::step_started("step.started", WorkflowRunStatus::Running)]
#[case::step_failed("step.failed", WorkflowRunStatus::Failed)]
#[case::hook_waiting("hook.waiting", WorkflowRunStatus::Paused)]
#[case::hook_received("hook.received", WorkflowRunStatus::Running)]
#[tokio::test]
async fn an_event_sets_the_run_status_its_type_maps_to(
    #[case] event_type: &str,
    #[case] status: WorkflowRunStatus,
) {
    let store = Arc::new(FakeStore::default());
    let run_id = store.seed_run(Some("hash-1"));
    let reply = post(
        Some(&store),
        Some(&member("hash-1")),
        &event_uri(&run_id),
        &an_event(event_type),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(store.status_updates(), vec![(run_id.clone(), status)]);
    assert_eq!(store.run_status(&run_id), Some(status.to_string()));
    assert_eq!(
        store.attempts(),
        vec![Attempt {
            table: SequenceTable::Events,
            sequence_number: 0,
            new_status: Some(status),
        }]
    );
}

#[tokio::test]
async fn an_event_with_an_unmapped_type_leaves_the_status_alone() {
    let store = Arc::new(FakeStore::default());
    let run_id = store.seed_run(Some("hash-1"));
    let reply = post(
        Some(&store),
        Some(&member("hash-1")),
        &event_uri(&run_id),
        &an_event("step.progress"),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(store.status_updates().is_empty());
    assert_eq!(store.run_status(&run_id).as_deref(), Some("running"));
    assert_eq!(store.attempts().first().unwrap().new_status, None);
}

#[tokio::test]
async fn an_event_asks_for_a_fresh_number_after_a_collision() {
    let store = Arc::new(FakeStore::default());
    let run_id = store.seed_run(Some("hash-1"));
    store.collide_events(1);
    let reply = post(
        Some(&store),
        Some(&member("hash-1")),
        &event_uri(&run_id),
        &an_event("step.started"),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let row: WorkflowEventRow = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(row.sequence_number, 1);
    let numbers: Vec<i32> = store
        .attempts()
        .iter()
        .map(|attempt| attempt.sequence_number)
        .collect();
    assert_eq!(numbers, [0, 1]);
    assert_eq!(store.event_writes().len(), 1);
}

#[tokio::test]
async fn four_collisions_still_land_on_the_fifth_attempt() {
    let store = Arc::new(FakeStore::default());
    let run_id = store.seed_run(Some("hash-1"));
    store.collide_events(4);
    let reply = post(
        Some(&store),
        Some(&member("hash-1")),
        &event_uri(&run_id),
        &an_event("step.started"),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let row: WorkflowEventRow = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(row.sequence_number, 4);
    assert_eq!(store.attempts().len(), 5);
}

#[tokio::test]
async fn five_collisions_answer_409_without_a_sixth_attempt() {
    let store = Arc::new(FakeStore::default());
    let run_id = store.seed_run(Some("hash-1"));
    store.collide_events(5);
    let reply = post(
        Some(&store),
        Some(&member("hash-1")),
        &event_uri(&run_id),
        &an_event("step.started"),
    )
    .await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    let detail: DetailReply = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(detail.detail, CONFLICT_DETAIL);
    let numbers: Vec<i32> = store
        .attempts()
        .iter()
        .map(|attempt| attempt.sequence_number)
        .collect();
    assert_eq!(numbers, [0, 1, 2, 3, 4]);
    assert!(store.event_writes().is_empty());
    assert_eq!(store.run_status(&run_id).as_deref(), Some("running"));
}

#[tokio::test]
async fn an_event_write_failure_is_a_500_without_the_cause() {
    let store = Arc::new(FakeStore::failing_as(FailureKind::Write));
    store.seed_run(Some("hash-1"));
    let reply = post(
        Some(&store),
        Some(&member("hash-1")),
        "/v1/workflows/runs/run-0/events",
        &an_event("step.started"),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let detail: DetailReply = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(detail.detail, "Internal server error");
}

#[tokio::test]
async fn an_event_run_lookup_failure_is_a_500() {
    let store = Arc::new(FakeStore::failing_as(FailureKind::Read));
    let reply = post(
        Some(&store),
        Some(&master()),
        "/v1/workflows/runs/run-0/events",
        &an_event("step.started"),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[rstest]
#[case::owner(member("hash-1"), "run-0", StatusCode::OK)]
#[case::foreign_member(member("hash-2"), "run-0", StatusCode::NOT_FOUND)]
#[case::view_only_on_own(viewer("hash-1"), "run-0", StatusCode::OK)]
#[case::view_only_on_foreign(viewer("hash-9"), "run-0", StatusCode::NOT_FOUND)]
#[case::proxy_admin_on_foreign(proxy_admin("hash-9"), "run-0", StatusCode::OK)]
#[case::master_on_foreign(master(), "run-0", StatusCode::OK)]
#[case::missing_for_a_member(member("hash-1"), "run-404", StatusCode::NOT_FOUND)]
#[case::missing_for_the_master(master(), "run-404", StatusCode::NOT_FOUND)]
#[tokio::test]
async fn an_event_append_enforces_ownership_except_for_a_full_admin(
    #[case] caller: Principal,
    #[case] run_id: &str,
    #[case] status: StatusCode,
) {
    let store = Arc::new(FakeStore::default());
    store.seed_run(Some("hash-1"));
    let reply = post(
        Some(&store),
        Some(&caller),
        &event_uri(run_id),
        &an_event("step.started"),
    )
    .await;
    assert_eq!(reply.status, status);
    assert_eq!(
        store.attempts().len(),
        usize::from(status == StatusCode::OK)
    );
    if status == StatusCode::NOT_FOUND {
        let detail: DetailReply = serde_json::from_str(&reply.body).unwrap();
        assert_eq!(detail.detail, format!("Run '{run_id}' not found"));
    }
}

#[tokio::test]
async fn an_event_append_without_a_database_is_a_500_with_the_db_error() {
    let reply = post(
        None,
        Some(&master()),
        "/v1/workflows/runs/run-0/events",
        &an_event("step.started"),
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
async fn an_event_append_without_a_caller_is_unauthorized() {
    let store = Arc::new(FakeStore::default());
    let run_id = store.seed_run(Some("hash-1"));
    let reply = post(
        Some(&store),
        None,
        &event_uri(&run_id),
        &an_event("step.started"),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert!(store.attempts().is_empty());
}

#[rstest]
#[case::missing_type(r#"{"step_name":"build"}"#, "missing", &["body", "event_type"])]
#[case::missing_step(r#"{"event_type":"step.started"}"#, "missing", &["body", "step_name"])]
#[case::type_is_a_number(r#"{"event_type":3,"step_name":"build"}"#, "value_error", &["body", "event_type"])]
#[case::step_is_a_list(r#"{"event_type":"a","step_name":["x"]}"#, "value_error", &["body", "step_name"])]
#[case::data_is_a_list(r#"{"event_type":"a","step_name":"b","data":[1]}"#, "value_error", &["body", "data"])]
#[case::not_json("not json", "json_invalid", &["body"])]
#[case::second_value(r#"{"event_type":"a","step_name":"b"} {}"#, "json_invalid", &["body"])]
#[tokio::test]
async fn an_event_append_rejects_a_body_of_the_wrong_shape(
    #[case] body: &str,
    #[case] issue_type: &str,
    #[case] loc: &[&str],
) {
    let store = Arc::new(FakeStore::default());
    let run_id = store.seed_run(Some("hash-1"));
    let reply = post(
        Some(&store),
        Some(&member("hash-1")),
        &event_uri(&run_id),
        body,
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let parsed: ValidationReply = serde_json::from_str(&reply.body).unwrap();
    let issue = parsed.detail.first().unwrap();
    assert_eq!(issue.issue_type, issue_type);
    let expected: Vec<LocReply> = loc
        .iter()
        .map(|name| LocReply::Name((*name).to_owned()))
        .collect();
    assert_eq!(issue.loc, expected);
    assert!(store.attempts().is_empty());
}

#[tokio::test]
async fn an_event_append_with_a_bad_body_and_no_database_is_a_422() {
    let reply = post(
        None,
        Some(&master()),
        "/v1/workflows/runs/run-0/events",
        "{}",
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
}
