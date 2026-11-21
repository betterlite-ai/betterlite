use std::error::Error;
use std::sync::atomic::Ordering;
use std::time::Duration;

use betterlite_db::database_failures::deadline::{DbLookupDeadlineExceeded, DbLookupError};
use betterlite_db::database_faults::{DbError as DatabaseFault, ErrorType};
use betterlite_db::postgres_store::errors::DbError as NativeDbError;
use tokio::time::timeout;

use super::logs::Logs;
use super::support::{FailureKind, Harness, request, request_with, unavailable, user_with};
use crate::auth_identity::errors::AuthError;
use crate::auth_read::table::AuthReadError;
use crate::auth_user_lookup::error::UserLookupError;
use crate::auth_user_lookup::rows::UserRowError;

#[tokio::test]
async fn not_found_rethrown_without_log() {
    let harness = Harness::default();
    let logs = Logs::default();
    let _guard = logs.install();
    let lookup = harness.lookup();
    let result = lookup.get_user_object(request(Some("u1"))).await;
    match result {
        Err(UserLookupError::NotFound(AuthError::UserNotFound { user_id })) => {
            assert_eq!(user_id, "u1");
        }
        other => panic!("a missing user keeps its own error: {other:?}"),
    }
    assert!(logs.entries().is_empty());
}

#[tokio::test]
async fn unavailable_identity_preserved() {
    let harness = Harness::default();
    harness.store.failures.lock().unwrap().insert(
        "find_by_id",
        FailureKind::Io("connection refused".to_owned()),
    );
    let logs = Logs::default();
    let _guard = logs.install();
    let lookup = harness.lookup();
    let result = lookup.get_user_object(request(Some("u1"))).await;
    match result {
        Err(UserLookupError::Unavailable(error)) => {
            assert_eq!(error.to_string(), "connection refused");
        }
        other => panic!("a transport failure is returned unchanged: {other:?}"),
    }
    assert_eq!(logs.errors().len(), 1);
}

#[tokio::test]
async fn other_read_error_keeps_cause() {
    let harness = Harness::default();
    harness
        .store
        .failures
        .lock()
        .unwrap()
        .insert("find_by_id", FailureKind::Named("boom".to_owned()));
    let lookup = harness.lookup();
    let result = lookup.get_user_object(request(Some("u1"))).await;
    match result {
        Err(UserLookupError::UserMissing(failure)) => {
            assert!(
                failure.to_string().ends_with("Got error - boom"),
                "{failure}"
            );
            let retained = Error::source(&failure).expect("the retained cause");
            assert_eq!(retained.to_string(), "boom");
        }
        other => panic!("an ordinary read failure is rewritten: {other:?}"),
    }
}

#[tokio::test]
async fn user_new_guidance() {
    let harness = Harness::default();
    harness
        .store
        .failures
        .lock()
        .unwrap()
        .insert("find_by_id", FailureKind::Named("boom".to_owned()));
    let lookup = harness.lookup();
    let result = lookup.get_user_object(request(Some("u1"))).await;
    let rendered = result.as_ref().err().map(ToString::to_string);
    assert_eq!(
        rendered.as_deref(),
        Some(
            "User doesn't exist in db. 'user_id'=u1. Create user via `/user/new` call. Got error - boom"
        )
    );
}

#[tokio::test]
async fn budget_failure_logged_once() {
    let harness = Harness::default();
    harness
        .store
        .failures
        .lock()
        .unwrap()
        .insert("find_by_id", FailureKind::Named("boom".to_owned()));
    let logs = Logs::default();
    let _guard = logs.install();
    let lookup = harness.lookup();
    let result = lookup.get_user_object(request(Some("u1"))).await;
    assert!(matches!(result, Err(UserLookupError::UserMissing(_))));
    let errors = logs.errors();
    assert_eq!(errors.len(), 1);
    assert!(errors[0].message.contains("budget lookup failed for user"));
    let cause = errors[0].fields.get("cause").cloned().unwrap_or_default();
    assert!(cause.contains("boom"), "{cause}");
}

#[tokio::test]
async fn inside_backfill_failure_wrapped() {
    let harness = Harness::default();
    *harness.store.by_id.lock().unwrap() = Some(user_with("u1", None, None));
    harness.store.failures.lock().unwrap().insert(
        "backfill",
        FailureKind::Named("backfill refused".to_owned()),
    );
    let logs = Logs::default();
    let _guard = logs.install();
    let lookup = harness.lookup();
    let result = lookup
        .get_user_object(request_with(Some("u1"), None, Some("a@example.com")))
        .await;
    match result {
        Err(UserLookupError::UserMissing(failure)) => {
            assert!(
                failure
                    .to_string()
                    .starts_with("User doesn't exist in db. 'user_id'=u1."),
                "{failure}"
            );
            assert!(
                failure.to_string().contains("Got error - backfill refused"),
                "{failure}"
            );
        }
        other => panic!("a database-path backfill failure is wrapped: {other:?}"),
    }
    assert_eq!(logs.errors().len(), 1);
}

#[tokio::test]
async fn cancellation_not_wrapped() {
    let harness = Harness::default();
    harness.store.hang.store(true, Ordering::SeqCst);
    let logs = Logs::default();
    let _guard = logs.install();
    let lookup = harness.lookup();
    let outcome = timeout(
        Duration::from_millis(10),
        lookup.get_user_object(request(Some("u1"))),
    )
    .await;
    match outcome {
        Err(_) => {}
        Ok(value) => panic!("the timeout cancels the lookup: {value:?}"),
    }
    assert!(logs.entries().is_empty());
}

fn fault(error_type: ErrorType, message: &str) -> DatabaseFault {
    DatabaseFault {
        error_type,
        message: message.to_owned(),
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    }
}

#[test]
fn prisma_faults_are_unavailable() {
    let cause = Box::new(fault(ErrorType::Prisma, ""));
    assert!(unavailable(cause));
}

#[test]
fn transport_words_make_data_faults_unavailable() {
    let cause = Box::new(fault(ErrorType::Data, "connection refused"));
    assert!(unavailable(cause));
}

#[test]
fn engine_tracebacks_make_faults_unavailable() {
    let mut engine_fault = fault(ErrorType::Other, "");
    engine_fault.traceback_modules = vec!["prisma.engine.errors".to_owned()];
    assert!(unavailable(Box::new(engine_fault)));
}

#[test]
fn quiet_data_faults_are_missing() {
    let cause = Box::new(fault(ErrorType::Data, ""));
    assert!(!unavailable(cause));
}

#[test]
fn stale_plan_faults_are_unavailable() {
    let cause = Box::new(fault(
        ErrorType::Data,
        "cached plan must not change result type",
    ));
    assert!(unavailable(cause));
}

#[test]
fn lookup_deadlines_are_unavailable() {
    let deadline = DbLookupDeadlineExceeded {
        lookup: "user".to_owned(),
        deadline_seconds: 5.0_f64,
        cause: None,
    };
    assert!(unavailable(Box::new(DbLookupError::Deadline(deadline))));
}

#[test]
fn lookup_data_faults_are_missing() {
    let cause = DbLookupError::Database(fault(ErrorType::Data, ""));
    assert!(!unavailable(Box::new(cause)));
}

#[test]
fn repository_transport_errors_are_unavailable() {
    let native = NativeDbError::<UserRowError>::NoDbConnected;
    let cause = AuthReadError::Repository(native);
    assert!(unavailable(Box::new(cause)));
}

#[test]
fn repository_invalid_records_are_missing() {
    let native = NativeDbError::<UserRowError>::InvalidRecord {
        source: UserRowError::TooLarge { len: 1 },
    };
    let cause = AuthReadError::Repository(native);
    assert!(!unavailable(Box::new(cause)));
}
