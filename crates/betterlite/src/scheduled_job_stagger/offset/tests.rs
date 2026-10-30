#![cfg(test)]

use rstest::rstest;

use super::offset_seconds;

#[test]
fn zero_window_is_always_zero() {
    assert_eq!(offset_seconds("job", "replica-1", 0), 0);
}

#[rstest]
#[case::one_second(1)]
#[case::seven_seconds(7)]
#[case::one_minute(60)]
#[case::five_minutes(300)]
fn offset_is_within_window(#[case] window: u32) {
    let offset = offset_seconds("monthly_spend_report_job", "replica-1:4242", window);
    assert!(offset < window, "{offset} should be < {window}");
}

#[test]
fn same_inputs_are_deterministic() {
    let first = offset_seconds("job-a", "replica-1", 300);
    let second = offset_seconds("job-a", "replica-1", 300);
    assert_eq!(first, second);
}

#[test]
fn different_identity_usually_changes_the_offset() {
    let first = offset_seconds("job-a", "replica-1", 300);
    let second = offset_seconds("job-a", "replica-2", 300);
    assert_ne!(first, second);
}

#[test]
fn different_job_id_usually_changes_the_offset() {
    let first = offset_seconds("job-a", "replica-1", 300);
    let second = offset_seconds("job-b", "replica-1", 300);
    assert_ne!(first, second);
}
