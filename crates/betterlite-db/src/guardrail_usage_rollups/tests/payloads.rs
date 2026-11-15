use chrono::{DateTime, Utc};
use rstest::rstest;

use crate::guardrail_usage_rollups::{
    GuardrailAction, GuardrailSpendLog, guardrail_status_to_action, payload,
};

use super::support::{capture_logs, from_json};

const START: &str = r#""2026-01-02T03:04:05Z""#;

fn payload_with_start(start: &str) -> GuardrailSpendLog {
    from_json(&format!(
        r#"{{"request_id": "req-1", "startTime": {start}}}"#
    ))
}

fn stamp_of(start: &str) -> Option<(String, DateTime<Utc>)> {
    payload::payload_stamp(&payload_with_start(start)).map(|stamp| (stamp.date, stamp.instant))
}

#[rstest]
#[case::missing(None, GuardrailAction::Passed)]
#[case::empty(Some(""), GuardrailAction::Passed)]
#[case::success(Some("success"), GuardrailAction::Passed)]
#[case::not_run(Some("not_run"), GuardrailAction::NotRun)]
#[case::not_run_upper(Some("NOT_RUN"), GuardrailAction::NotRun)]
#[case::not_running(Some("not_running"), GuardrailAction::Passed)]
#[case::intervened(Some("intervened"), GuardrailAction::Blocked)]
#[case::blocked(Some("blocked_by_policy"), GuardrailAction::Blocked)]
#[case::flagged(Some("flagged"), GuardrailAction::Flagged)]
#[case::failed(Some("failed"), GuardrailAction::Flagged)]
#[case::error(Some("error"), GuardrailAction::Flagged)]
#[case::unknown(Some("maybe"), GuardrailAction::Passed)]
fn maps_a_guardrail_status_to_its_action(
    #[case] status: Option<&str>,
    #[case] expected: GuardrailAction,
) {
    assert_eq!(guardrail_status_to_action(status), expected);
}

#[rstest]
#[case::rfc3339_utc(r#""2026-01-02T03:04:05Z""#, Some(("2026-01-02", 1_767_323_045)))]
#[case::rfc3339_offset(r#""2026-01-02T01:04:05+04:00""#, Some(("2026-01-01", 1_767_301_445)))]
#[case::naive(r#""2026-01-02T03:04:05""#, Some(("2026-01-02", 1_767_323_045)))]
#[case::naive_micros(r#""2026-01-02T03:04:05.123456""#, Some(("2026-01-02", 1_767_323_045)))]
#[case::space(r#""2026-01-02 03:04:05""#, Some(("2026-01-02", 1_767_323_045)))]
#[case::python_text(r#""2026-01-02 03:04:05+00:00""#, Some(("2026-01-02", 1_767_323_045)))]
#[case::database_text(r#""2026-01-02 01:04:05+04:00""#, Some(("2026-01-01", 1_767_301_445)))]
#[case::minutes(r#""2026-01-02T03:04""#, Some(("2026-01-02", 1_767_323_040)))]
#[case::date_only(r#""2026-01-02""#, Some(("2026-01-02", 1_767_312_000)))]
#[case::garbage(r#""yesterday""#, None)]
#[case::null("null", None)]
fn reads_the_utc_date_from_the_payload_start_time(
    #[case] start: &str,
    #[case] expected: Option<(&str, i64)>,
) {
    assert_eq!(
        stamp_of(start),
        expected.map(|(date, instant)| {
            (
                date.to_owned(),
                DateTime::from_timestamp(instant, 0).expect("a test instant"),
            )
        })
    );
}

#[rstest]
#[case::no_request_id(r#"{"startTime": "2026-01-02T03:04:05Z"}"#)]
#[case::empty_request_id(r#"{"request_id": "", "startTime": "2026-01-02T03:04:05Z"}"#)]
#[case::no_start_time(r#"{"request_id": "req-1"}"#)]
fn skips_a_payload_without_a_request_id_and_a_usable_start_time(#[case] json: &str) {
    assert!(payload::payload_stamp(&from_json(json)).is_none());
}

#[test]
fn rejects_a_start_time_that_is_not_a_string() {
    assert!(serde_json::from_str::<GuardrailSpendLog>(r#"{"startTime": 5}"#).is_err());
}

#[test]
fn reads_entries_from_an_inline_metadata_object() {
    let payload = from_json(
        r#"{"metadata": {"guardrail_information": [{"guardrail_id": "g1", "guardrail_status": "success"}]}}"#,
    );
    let entries = payload::information_entries(&payload);
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries
            .first()
            .and_then(|entry| entry.guardrail_id.as_deref()),
        Some("g1")
    );
}

#[test]
fn reads_entries_from_metadata_held_in_a_json_string() {
    let payload =
        from_json(r#"{"metadata": "{\"guardrail_information\": [{\"guardrail_id\": \"g1\"}]}"}"#);
    let entries = payload::information_entries(&payload);
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries
            .first()
            .and_then(|entry| entry.guardrail_id.as_deref()),
        Some("g1")
    );
}

#[test]
fn prefers_the_guardrail_information_list_over_the_standard_logging_one() {
    let payload = from_json(
        r#"{"metadata": {"guardrail_information": [{"guardrail_id": "first"}],
             "standard_logging_guardrail_information": [{"guardrail_id": "second"}]}}"#,
    );
    let entries = payload::information_entries(&payload);
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries
            .first()
            .and_then(|entry| entry.guardrail_id.as_deref()),
        Some("first")
    );
}

#[test]
fn falls_back_to_the_standard_logging_list_when_the_first_is_empty() {
    let payload = from_json(
        r#"{"metadata": {"guardrail_information": [],
             "standard_logging_guardrail_information": [{"guardrail_id": "second"}]}}"#,
    );
    let entries = payload::information_entries(&payload);
    assert_eq!(
        entries
            .first()
            .and_then(|entry| entry.guardrail_id.as_deref()),
        Some("second")
    );
}

#[rstest]
#[case::no_metadata(r#"{"request_id": "req-1"}"#)]
#[case::null_metadata(r#"{"metadata": null}"#)]
#[case::empty_metadata(r#"{"metadata": {}}"#)]
#[case::other_keys(r#"{"metadata": {"spend_logs_metadata": {"a": 1}}}"#)]
fn reads_no_entries_from_metadata_without_a_guardrail_list(#[case] json: &str) {
    assert!(payload::information_entries(&from_json(json)).is_empty());
}

#[rstest]
#[case::not_json(r#"{"metadata": "not json"}"#)]
#[case::a_json_list(r#"{"metadata": "[]"}"#)]
#[case::a_json_null(r#"{"metadata": "null"}"#)]
fn warns_and_reads_no_entries_from_a_metadata_string_that_is_not_an_object(#[case] json: &str) {
    let capture = capture_logs();
    let payload = from_json(json);
    assert!(payload::information_entries(&payload).is_empty());
    assert!(
        capture
            .logs
            .has("WARN", "metadata text is not a JSON object")
    );
}

#[test]
fn rejects_metadata_whose_guardrail_information_is_not_a_list() {
    assert!(
        serde_json::from_str::<GuardrailSpendLog>(r#"{"metadata": {"guardrail_information": 5}}"#)
            .is_err()
    );
}

#[test]
fn resolves_an_entry_without_an_id_through_the_name_to_id_map() {
    let payload = from_json(&format!(
        r#"{{"request_id": "req-1", "startTime": {START}, "metadata": {{"guardrail_information": [
            {{"guardrail_name": "pii", "guardrail_id": "g1", "guardrail_status": "success"}},
            {{"guardrail_name": "pii", "guardrail_status": "success"}}
        ]}}}}"#
    ));
    let entries = payload::information_entries(&payload);
    let ids = payload::ids_by_name(entries);
    let resolved: Vec<Option<String>> = entries
        .iter()
        .map(|entry| payload::resolved_id(entry, &ids))
        .collect();
    assert_eq!(
        resolved,
        [Some(String::from("g1")), Some(String::from("g1"))]
    );
}
