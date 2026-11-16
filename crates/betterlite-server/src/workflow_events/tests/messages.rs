use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use rstest::rstest;

use super::support::{
    DetailReply, FailureKind, FakeStore, LocReply, ValidationReply, master, member, post,
    proxy_admin, viewer,
};
use crate::workflow_events::error::CONFLICT_DETAIL;
use crate::workflow_events::models::MessageCreateData;
use crate::workflow_runs::models::WorkflowMessageRow;

fn message_uri(run_id: &str) -> String {
    format!("/v1/workflows/runs/{run_id}/messages")
}

fn a_message() -> String {
    r#"{"role":"assistant","content":"the build passed"}"#.to_owned()
}

#[tokio::test]
async fn a_message_appends_after_the_highest_sequence_of_its_run() {
    let store = Arc::new(FakeStore::default());
    let run_id = store.seed_run(Some("hash-1"));
    store.seed_message(&run_id, 0);
    store.seed_message(&run_id, 3);
    let reply = post(
        Some(&store),
        Some(&member("hash-1")),
        &message_uri(&run_id),
        &a_message(),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let row: WorkflowMessageRow = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(row.sequence_number, 4);
    assert_eq!(row.run_id, run_id);
    assert_eq!(row.role, "assistant");
    assert_eq!(row.content, "the build passed");
    assert_eq!(row.session_id, None);
    assert_eq!(row.message_id, format!("message-{run_id}-4"));
}

#[tokio::test]
async fn a_message_has_its_own_sequence_beside_the_event_log() {
    let store = Arc::new(FakeStore::default());
    let run_id = store.seed_run(Some("hash-1"));
    store.seed_event(&run_id, 0);
    store.seed_event(&run_id, 1);
    store.seed_event(&run_id, 2);
    let reply = post(
        Some(&store),
        Some(&member("hash-1")),
        &message_uri(&run_id),
        &a_message(),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let row: WorkflowMessageRow = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(row.sequence_number, 0);
}

#[tokio::test]
async fn a_message_keeps_its_session_id() {
    let store = Arc::new(FakeStore::default());
    let run_id = store.seed_run(Some("hash-1"));
    let body = r#"{"role":"user","content":"resume","session_id":"conversation-one"}"#;
    let reply = post(
        Some(&store),
        Some(&member("hash-1")),
        &message_uri(&run_id),
        body,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let written = store.message_writes().first().unwrap().clone();
    assert_eq!(written.session_id.as_deref(), Some("conversation-one"));
    let row: WorkflowMessageRow = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(row.session_id.as_deref(), Some("conversation-one"));
}

#[tokio::test]
async fn a_message_without_a_session_stores_none() {
    let store = Arc::new(FakeStore::default());
    let run_id = store.seed_run(Some("hash-1"));
    let reply = post(
        Some(&store),
        Some(&member("hash-1")),
        &message_uri(&run_id),
        &a_message(),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(store.message_writes().first().unwrap().session_id, None);
}

#[tokio::test]
async fn a_message_stores_its_content_in_full() {
    let store = Arc::new(FakeStore::default());
    let run_id = store.seed_run(Some("hash-1"));
    let content = "line one\nline two, ünïcode and a tab\t".repeat(64);
    let body = format!(
        r#"{{"role":"user","content":{}}}"#,
        serde_json::to_string(&content).unwrap()
    );
    let reply = post(
        Some(&store),
        Some(&member("hash-1")),
        &message_uri(&run_id),
        &body,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        store.message_writes(),
        vec![MessageCreateData {
            run_id: run_id.clone(),
            role: "user".to_owned(),
            content: content.clone(),
            sequence_number: 0,
            session_id: None,
        }]
    );
    let row: WorkflowMessageRow = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(row.content, content);
}

#[tokio::test]
async fn a_message_leaves_the_run_status_alone() {
    let store = Arc::new(FakeStore::default());
    let run_id = store.seed_run(Some("hash-1"));
    let reply = post(
        Some(&store),
        Some(&member("hash-1")),
        &message_uri(&run_id),
        r#"{"role":"user","content":"step.failed"}"#,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(store.status_updates().is_empty());
    assert_eq!(store.run_status(&run_id).as_deref(), Some("running"));
}

#[tokio::test]
async fn a_message_asks_for_a_fresh_number_after_a_collision() {
    let store = Arc::new(FakeStore::default());
    let run_id = store.seed_run(Some("hash-1"));
    store.collide_messages(2);
    let reply = post(
        Some(&store),
        Some(&member("hash-1")),
        &message_uri(&run_id),
        &a_message(),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let row: WorkflowMessageRow = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(row.sequence_number, 2);
    let numbers: Vec<i32> = store
        .attempts()
        .iter()
        .map(|attempt| attempt.sequence_number)
        .collect();
    assert_eq!(numbers, [0, 1, 2]);
    assert_eq!(store.message_writes().len(), 1);
}

#[tokio::test]
async fn five_message_collisions_answer_409_without_a_sixth_attempt() {
    let store = Arc::new(FakeStore::default());
    let run_id = store.seed_run(Some("hash-1"));
    store.collide_messages(5);
    let reply = post(
        Some(&store),
        Some(&member("hash-1")),
        &message_uri(&run_id),
        &a_message(),
    )
    .await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    let detail: DetailReply = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(detail.detail, CONFLICT_DETAIL);
    assert_eq!(store.attempts().len(), 5);
    assert!(store.message_writes().is_empty());
}

#[tokio::test]
async fn a_message_write_failure_is_a_500_without_the_cause() {
    let store = Arc::new(FakeStore::failing_as(FailureKind::Write));
    store.seed_run(Some("hash-1"));
    let reply = post(
        Some(&store),
        Some(&member("hash-1")),
        "/v1/workflows/runs/run-0/messages",
        &a_message(),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let detail: DetailReply = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(detail.detail, "Internal server error");
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
async fn a_message_append_enforces_ownership_except_for_a_full_admin(
    #[case] caller: Principal,
    #[case] run_id: &str,
    #[case] status: StatusCode,
) {
    let store = Arc::new(FakeStore::default());
    store.seed_run(Some("hash-1"));
    let reply = post(
        Some(&store),
        Some(&caller),
        &message_uri(run_id),
        &a_message(),
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
async fn a_message_append_without_a_database_is_a_500_with_the_db_error() {
    let reply = post(
        None,
        Some(&master()),
        "/v1/workflows/runs/run-0/messages",
        &a_message(),
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
async fn a_message_append_without_a_caller_is_unauthorized() {
    let store = Arc::new(FakeStore::default());
    let run_id = store.seed_run(Some("hash-1"));
    let reply = post(Some(&store), None, &message_uri(&run_id), &a_message()).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert!(store.attempts().is_empty());
}

#[rstest]
#[case::missing_role(r#"{"content":"hi"}"#, "missing", &["body", "role"])]
#[case::missing_content(r#"{"role":"user"}"#, "missing", &["body", "content"])]
#[case::role_is_a_number(r#"{"role":7,"content":"hi"}"#, "value_error", &["body", "role"])]
#[case::content_is_a_list(r#"{"role":"user","content":["hi"]}"#, "value_error", &["body", "content"])]
#[case::session_is_a_number(
    r#"{"role":"user","content":"hi","session_id":3}"#,
    "value_error",
    &["body", "session_id"]
)]
#[case::not_json("not json", "json_invalid", &["body"])]
#[case::second_value(r#"{"role":"user","content":"hi"} {}"#, "json_invalid", &["body"])]
#[tokio::test]
async fn a_message_append_rejects_a_body_of_the_wrong_shape(
    #[case] body: &str,
    #[case] issue_type: &str,
    #[case] loc: &[&str],
) {
    let store = Arc::new(FakeStore::default());
    let run_id = store.seed_run(Some("hash-1"));
    let reply = post(
        Some(&store),
        Some(&member("hash-1")),
        &message_uri(&run_id),
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
async fn a_message_append_with_a_bad_body_and_no_database_is_a_422() {
    let reply = post(
        None,
        Some(&master()),
        "/v1/workflows/runs/run-0/messages",
        "{}",
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
}
