use std::sync::{Arc, Mutex};

use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::Role;
use rstest::rstest;

use super::support::{FakeStore, principal};
use crate::auth::auth_key_lookup::tests::master_caller;
use crate::tracing_routes::access::TraceRole;
use crate::workflow_runs::access::{RunCaller, require_run};
use crate::workflow_runs::error::RunsError;
use crate::workflow_runs::models::{
    EVENT_STATUS_MAP, MAX_SEQUENCE_RETRIES, SequenceTable, WorkflowRunStatus,
};
use crate::workflow_runs::sequence::next_sequence_number;

fn caller(role: TraceRole, key: Option<&str>) -> RunCaller {
    RunCaller {
        role,
        key: key.map(str::to_owned),
    }
}

fn keyed(key_id: Option<&str>, token_id: Option<&str>) -> Principal {
    let mut built = principal("virtual-key", vec![Role::TeamMember], token_id);
    built.credential_ref.key_id = key_id.map(str::to_owned);
    built
}

#[test]
fn the_sequence_retry_limit_is_five() {
    assert_eq!(MAX_SEQUENCE_RETRIES, 5);
}

#[test]
fn events_map_to_the_four_status_transitions() {
    assert_eq!(
        EVENT_STATUS_MAP,
        [
            ("step.started", WorkflowRunStatus::Running),
            ("step.failed", WorkflowRunStatus::Failed),
            ("hook.waiting", WorkflowRunStatus::Paused),
            ("hook.received", WorkflowRunStatus::Running),
        ]
    );
}

#[rstest]
#[case::pending(WorkflowRunStatus::Pending, "pending")]
#[case::running(WorkflowRunStatus::Running, "running")]
#[case::paused(WorkflowRunStatus::Paused, "paused")]
#[case::completed(WorkflowRunStatus::Completed, "completed")]
#[case::failed(WorkflowRunStatus::Failed, "failed")]
fn a_status_is_its_lowercase_name(#[case] status: WorkflowRunStatus, #[case] text: &str) {
    assert_eq!(status.to_string(), text);
    assert_eq!(
        serde_json::to_string(&status).unwrap(),
        format!("\"{text}\"")
    );
}

#[rstest]
#[case::master_key(master_caller().principal, TraceRole::Admin, Some("litellm_proxy_master_key"))]
#[case::platform_admin(principal("virtual-key", vec![Role::PlatformAdmin], Some("hash-1")), TraceRole::Admin, Some("hash-1"))]
#[case::platform_viewer(principal("virtual-key", vec![Role::PlatformViewer], Some("hash-1")), TraceRole::AdminViewOnly, Some("hash-1"))]
#[case::admin_and_viewer(principal("virtual-key", vec![Role::PlatformViewer, Role::PlatformAdmin], None), TraceRole::Admin, None)]
#[case::team_member(principal("virtual-key", vec![Role::TeamMember], Some("hash-1")), TraceRole::Other, Some("hash-1"))]
#[case::no_roles(
    principal("virtual-key", Vec::new(), Some("hash-1")),
    TraceRole::Other,
    Some("hash-1")
)]
#[case::empty_token(principal("virtual-key", vec![Role::TeamMember], Some("")), TraceRole::Other, None)]
#[case::key_id_fallback(keyed(Some("key-7"), None), TraceRole::Other, Some("key-7"))]
#[case::token_wins_over_key_id(
    keyed(Some("key-7"), Some("hash-1")),
    TraceRole::Other,
    Some("hash-1")
)]
fn the_caller_comes_from_the_principal(
    #[case] who: Principal,
    #[case] role: TraceRole,
    #[case] key: Option<&str>,
) {
    assert_eq!(RunCaller::from_principal(&who), caller(role, key));
}

#[rstest]
#[case::proxy_admin(TraceRole::Admin, true)]
#[case::view_only(TraceRole::AdminViewOnly, true)]
#[case::other(TraceRole::Other, false)]
fn only_the_admin_roles_have_admin_view(#[case] role: TraceRole, #[case] admin_view: bool) {
    let who = caller(role, Some("hash-1"));
    assert_eq!(who.has_admin_view(), admin_view);
    assert_eq!(who.read_scope().is_none(), admin_view);
    assert_eq!(
        who.read_scope().is_some_and(|scoped| *scoped == who),
        !admin_view
    );
}

#[rstest]
#[case::no_scope(None, Some("hash-2"), true)]
#[case::proxy_admin_foreign(Some(caller(TraceRole::Admin, Some("hash-9"))), Some("hash-2"), true)]
#[case::proxy_admin_without_key(Some(caller(TraceRole::Admin, None)), Some("hash-2"), true)]
#[case::owner(Some(caller(TraceRole::Other, Some("hash-2"))), Some("hash-2"), true)]
#[case::view_only_owner(
    Some(caller(TraceRole::AdminViewOnly, Some("hash-2"))),
    Some("hash-2"),
    true
)]
#[case::view_only_foreign(
    Some(caller(TraceRole::AdminViewOnly, Some("hash-9"))),
    Some("hash-2"),
    false
)]
#[case::other_key(Some(caller(TraceRole::Other, Some("hash-9"))), Some("hash-2"), false)]
#[case::member_without_key(Some(caller(TraceRole::Other, None)), Some("hash-2"), false)]
#[case::run_without_owner(Some(caller(TraceRole::Other, Some("hash-9"))), None, false)]
#[case::neither_has_a_key(Some(caller(TraceRole::Other, None)), None, false)]
#[tokio::test]
async fn ownership_is_checked_for_everyone_but_a_full_proxy_admin(
    #[case] scope: Option<RunCaller>,
    #[case] created_by: Option<&str>,
    #[case] allowed: bool,
) {
    let store = FakeStore::default();
    let run_id = store.seed_run(created_by, "review", "running");
    let found = require_run(&store, &run_id, scope.as_ref()).await;
    match (allowed, found) {
        (true, Ok(row)) => assert_eq!(row.run_id, run_id),
        (false, Err(RunsError::NotFound { run_id: missing })) => assert_eq!(missing, run_id),
        (_, other) => panic!("unexpected result {other:?}"),
    }
}

#[tokio::test]
async fn require_run_reports_a_missing_run_for_any_caller() {
    let store = FakeStore::default();
    let found = require_run(&store, "run-404", None).await;
    assert!(matches!(found, Err(RunsError::NotFound { run_id }) if run_id == "run-404"));
}

#[tokio::test]
async fn require_run_passes_a_store_failure_on() {
    let store = FakeStore {
        state: Mutex::default(),
        failing: true,
        vanish_on_update: false,
    };
    let found = require_run(&store, "run-0", None).await;
    assert!(matches!(found, Err(RunsError::Store(_))));
}

#[rstest]
#[case::first_event(SequenceTable::Events, &[], &[], 0_i32)]
#[case::after_events(SequenceTable::Events, &[0_i32, 1_i32, 4_i32], &[], 5_i32)]
#[case::first_message(SequenceTable::Messages, &[3_i32], &[], 0_i32)]
#[case::after_messages(SequenceTable::Messages, &[9_i32], &[0_i32, 2_i32], 3_i32)]
#[tokio::test]
async fn the_next_sequence_number_follows_the_highest_of_that_table(
    #[case] table: SequenceTable,
    #[case] events: &[i32],
    #[case] messages: &[i32],
    #[case] expected: i32,
) {
    let store = Arc::new(FakeStore::default());
    let run_id = store.seed_run(None, "review", "running");
    let other = store.seed_run(None, "review", "running");
    store.seed_sequences(&run_id, events, messages);
    store.seed_event(&other, 40);
    store.seed_message(&other, 50);
    let port: &FakeStore = &store;
    assert_eq!(
        next_sequence_number(port, &run_id, table).await.unwrap(),
        expected
    );
}

#[tokio::test]
async fn the_next_sequence_number_refuses_to_overflow() {
    let store = FakeStore::default();
    let run_id = store.seed_run(None, "review", "running");
    store.seed_event(&run_id, i32::MAX);
    let next = next_sequence_number(&store, &run_id, SequenceTable::Events).await;
    assert!(matches!(next, Err(RunsError::SequenceOverflow)));
}

#[tokio::test]
async fn the_next_sequence_number_passes_a_store_failure_on() {
    let store = FakeStore {
        state: Mutex::default(),
        failing: true,
        vanish_on_update: false,
    };
    let next = next_sequence_number(&store, "run-0", SequenceTable::Messages).await;
    assert!(matches!(next, Err(RunsError::Store(_))));
}
