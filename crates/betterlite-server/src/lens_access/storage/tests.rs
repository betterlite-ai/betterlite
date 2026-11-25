#![cfg(test)]

use axum::http::StatusCode;

use crate::lens_access::error::failure_text;
use crate::lens_access::storage::source_reader;
use crate::lens_access::tests::support::FakeStorage;

#[test]
fn without_storage_the_reader_helper_is_501() {
    let error = source_reader::<FakeStorage>(None)
        .err()
        .expect("tracing is off");
    assert_eq!(error.status(), StatusCode::NOT_IMPLEMENTED);
    assert_eq!(
        failure_text(&error),
        "Agent tracing is not enabled. Set `tracing:` in general_settings and CLICKHOUSE_URL."
    );
}

#[test]
fn with_storage_the_reader_helper_returns_a_reader() {
    let storage = FakeStorage::default();
    let reader = source_reader(Some(&storage)).expect("tracing is on");
    assert!(reader.storage.samples().is_empty());
}
