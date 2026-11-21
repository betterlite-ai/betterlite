use axum::http::StatusCode;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;

use super::support::{FakeRows, caller, store_failure};
use crate::memory_records::error::{internal_error, require_database};
use crate::memory_records::store::{
    MemoryLookupError, find_memory_for_caller, find_visible_memory_or_none,
};
use crate::python_hooks::test_support::capture;

#[test]
fn missing_database_500() {
    let error = require_database::<u8>(None).unwrap_err();
    assert_eq!(error.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        error.detail,
        CommonProxyErrors::DbNotConnectedError.as_str()
    );
    assert_eq!(require_database(Some(7_u8)).unwrap(), 7);
}

#[tokio::test]
async fn lookup_failure_logs_and_500() {
    let (captured, guard) = capture();
    let alice = caller(Some("alice"), None, None);
    let lookup = find_memory_for_caller(&FakeRows::failing(), "note", &alice).await;
    let source = match lookup {
        Ok(_) => panic!("the failing store cannot answer"),
        Err(MemoryLookupError::Http(_)) => panic!("a store failure is not an HTTP error"),
        Err(MemoryLookupError::Rows(error)) => error,
    };
    let outcome = internal_error(
        "Error upserting memory",
        &source,
        "Internal error updating memory entry.",
    );
    assert_eq!(outcome.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(outcome.detail, "Internal error updating memory entry.");
    let log = captured.text();
    assert!(log.contains("Error upserting memory"));
    assert!(log.contains("No DB Connected"));
    drop(guard);
}

#[test]
fn create_failure_logs_and_500() {
    let (captured, guard) = capture();
    let source = store_failure();
    let outcome = internal_error(
        "Error creating memory",
        &source,
        "Internal error creating memory entry.",
    );
    assert_eq!(outcome.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(outcome.detail, "Internal error creating memory entry.");
    assert!(!outcome.detail.contains("No DB Connected"));
    let log = captured.text();
    assert!(log.contains("Error creating memory"));
    assert!(log.contains("No DB Connected"));
    drop(guard);
}

#[test]
fn update_failure_logs_and_500() {
    let (captured, guard) = capture();
    let source = store_failure();
    let outcome = internal_error(
        "Error upserting memory",
        &source,
        "Internal error updating memory entry.",
    );
    assert_eq!(outcome.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(outcome.detail, "Internal error updating memory entry.");
    assert!(!outcome.detail.contains("No DB Connected"));
    let log = captured.text();
    assert!(log.contains("Error upserting memory"));
    drop(guard);
}

#[test]
fn delete_failure_logs_and_500() {
    let (captured, guard) = capture();
    let source = store_failure();
    let outcome = internal_error(
        "Error deleting memory",
        &source,
        "Internal error deleting memory entry.",
    );
    assert_eq!(outcome.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(outcome.detail, "Internal error deleting memory entry.");
    assert!(!outcome.detail.contains("No DB Connected"));
    let log = captured.text();
    assert!(log.contains("Error deleting memory"));
    drop(guard);
}

#[tokio::test]
async fn visibility_helper_suppresses_404_only() {
    let alice = caller(Some("alice"), None, None);

    let missing = find_visible_memory_or_none(&FakeRows::new(None), "note", &alice)
        .await
        .unwrap();
    assert_eq!(missing, None);

    let refused = find_visible_memory_or_none(&FakeRows::forbidden(), "note", &alice).await;
    match refused {
        Ok(_) => panic!("a 403 is not a missing row"),
        Err(MemoryLookupError::Rows(_)) => panic!("the source refused with an HTTP status"),
        Err(MemoryLookupError::Http(error)) => {
            assert_eq!(error.status, StatusCode::FORBIDDEN);
        }
    }
}
