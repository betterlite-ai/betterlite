use rstest::rstest;

use super::super::async_invoke;
use super::{SECOND, async_result, status_response};
use crate::api::api_boundaries::ApiLimits;

#[rstest]
#[case::completed_stamps_the_end("Completed", Some(SECOND + 120), None, None)]
#[case::failed_stamps_the_failure("Failed", None, Some(SECOND + 120), None)]
#[case::in_progress_stamps_the_change("InProgress", None, None, Some(SECOND + 60))]
#[case::validating_stamps_the_change("Validating", None, None, Some(SECOND + 60))]
#[case::scheduled_stamps_the_change("Scheduled", None, None, Some(SECOND + 60))]
#[case::lowercase_completed_keeps_no_end("completed", None, None, None)]
#[case::partially_completed_stamps_completion("PartiallyCompleted", Some(SECOND + 120), None, None)]
fn async_invoke_timestamps(
    #[case] raw: &str,
    #[case] completed_at: Option<i64>,
    #[case] failed_at: Option<i64>,
    #[case] in_progress_at: Option<i64>,
) {
    let batch = async_result(&status_response(raw)).batch;
    assert_eq!(batch.completed_at, completed_at);
    assert_eq!(batch.failed_at, failed_at);
    assert_eq!(batch.in_progress_at, in_progress_at);
}

#[test]
fn async_invoke_without_a_submit_time_uses_the_clock() {
    let mut status = status_response("Completed");
    status.submit_time = None;
    let batch = async_invoke::normalize(&status, SECOND, &ApiLimits::default())
        .expect("normalized")
        .batch;
    assert_eq!(batch.created_at, SECOND);
}

#[test]
fn async_invoke_reads_only_rfc_3339_timestamps() {
    let mut status = status_response("Completed");
    status.submit_time = Some("2024-01-01T00:00:00".to_owned());
    status.end_time = Some("2024-01-01T00:02:00".to_owned());
    let batch = async_result(&status).batch;
    assert_eq!(batch.created_at, SECOND);
    assert_eq!(batch.completed_at, None);
}

#[test]
fn async_invoke_rejects_an_oversized_failure_message() {
    let mut status = status_response("Failed");
    status.failure_message =
        Some("x".repeat(ApiLimits::default().max_event_bytes.saturating_add(1)));
    let error = async_invoke::normalize(&status, SECOND, &ApiLimits::default())
        .expect_err("the failure message is too large");
    assert_eq!(error.fields().code, "event_too_large");
}
