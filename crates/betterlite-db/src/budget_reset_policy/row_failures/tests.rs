#![cfg(test)]

use super::{RowFailure, RowFailures};

#[test]
fn row_failures_summary_is_none_without_failures() {
    let failures = RowFailures::default();
    assert_eq!(failures.summary("keys"), None);
    assert!(failures.rows.is_empty());
}

#[test]
fn row_failures_summary_counts_and_names_each_row() {
    let mut failures = RowFailures::default();
    failures.record("k1", "boom");
    failures.record("k2", "timeout");
    assert_eq!(failures.rows.len(), 2);
    assert_eq!(
        failures.summary("keys"),
        Some("Failed to reset 2 keys: k1: boom; k2: timeout".to_owned())
    );
}

#[test]
fn row_failures_keeps_the_row_and_error_of_each_failure() {
    let mut failures = RowFailures::default();
    failures.record("user-1", "row is poisoned");
    assert_eq!(
        failures.rows,
        [RowFailure {
            row: "user-1".to_owned(),
            error: "row is poisoned".to_owned(),
        }]
    );
}

#[test]
fn row_failures_summary_of_one_row_uses_the_singular_subject() {
    let mut failures = RowFailures::default();
    failures.record("team-1", "write refused");
    assert_eq!(
        failures.summary("team"),
        Some("Failed to reset 1 team: team-1: write refused".to_owned())
    );
}
