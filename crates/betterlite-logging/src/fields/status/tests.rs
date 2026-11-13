#![cfg(test)]

use rstest::rstest;

use super::{GuardrailStatus, LoggingStatus, StatusFields, get_status_fields};
use crate::fields::payload::records::GuardrailInformation;

fn entry(status: &str) -> GuardrailInformation {
    let encoded = format!(r#"{{"guardrail_status":"{status}"}}"#);
    serde_json::from_str(&encoded).unwrap()
}

#[rstest]
#[case::no_information(None, LoggingStatus::Success, GuardrailStatus::NotRun)]
#[case::empty_list(Some(Vec::new()), LoggingStatus::Failure, GuardrailStatus::NotRun)]
#[case::success_only(Some(vec![entry("success")]), LoggingStatus::Success, GuardrailStatus::Success)]
#[case::blocked_wins_over_success(
    Some(vec![entry("success"), entry("blocked")]),
    LoggingStatus::Success,
    GuardrailStatus::GuardrailIntervened
)]
#[case::flagged_beats_success(
    Some(vec![entry("success"), entry("guardrail_flagged")]),
    LoggingStatus::Success,
    GuardrailStatus::GuardrailFlagged
)]
#[case::legacy_failure(
    Some(vec![entry("not_run"), entry("failure")]),
    LoggingStatus::Failure,
    GuardrailStatus::GuardrailFailedToRespond
)]
#[case::failed_to_respond_under_intervened(
    Some(vec![entry("guardrail_intervened"), entry("guardrail_failed_to_respond")]),
    LoggingStatus::Success,
    GuardrailStatus::GuardrailIntervened
)]
#[case::unknown_status_is_not_run(
    Some(vec![entry("surprise")]),
    LoggingStatus::Success,
    GuardrailStatus::NotRun
)]
#[case::missing_status_is_not_run(
    Some(vec![GuardrailInformation::default()]),
    LoggingStatus::Success,
    GuardrailStatus::NotRun
)]
fn logging_fields_behavior_tests_behavior(
    #[case] guardrail_information: Option<Vec<GuardrailInformation>>,
    #[case] status: LoggingStatus,
    #[case] expected: GuardrailStatus,
) {
    let actual = get_status_fields(status, guardrail_information.as_deref());
    assert_eq!(
        actual,
        StatusFields {
            llm_api_status: status,
            guardrail_status: expected,
        }
    );
}

#[test]
fn a_guardrail_entry_that_is_not_an_object_is_rejected_at_the_boundary() {
    let parsed = serde_json::from_str::<Vec<GuardrailInformation>>(
        r#"[{"guardrail_status":"success"}, "x"]"#,
    );
    let _error = parsed.unwrap_err();
}

#[test]
fn a_guardrail_status_that_is_not_text_is_rejected_at_the_boundary() {
    let parsed = serde_json::from_str::<Vec<GuardrailInformation>>(r#"[{"guardrail_status":5}]"#);
    let _error = parsed.unwrap_err();
}
