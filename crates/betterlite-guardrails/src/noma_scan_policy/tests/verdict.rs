use rstest::rstest;
use tracing::subscriber::with_default;

use crate::guardrail_records::record::GuardrailStatus;
use crate::noma_scan_policy::error::NomaError;
use crate::noma_scan_policy::verdict::{
    check_verdict, determine_guardrail_status, extract_anonymized_content,
    handle_verdict_background, only_sensitive_data_failed, should_anonymize,
};
use crate::noma_scan_policy::{MessageRole, NomaSettings};

use super::support::{
    CapturedLog, anonymizing_settings, data, monitoring_settings, response, settings,
};

#[rstest]
#[case::missing(r#"{"scanResult": []}"#, GuardrailStatus::Success)]
#[case::explicit_false(r#"{"aggregatedScanResult": false}"#, GuardrailStatus::Success)]
#[case::explicit_true(
    r#"{"aggregatedScanResult": true}"#,
    GuardrailStatus::GuardrailIntervened
)]
#[case::explicit_null(
    r#"{"aggregatedScanResult": null}"#,
    GuardrailStatus::GuardrailFailedToRespond
)]
#[case::string_value(
    r#"{"aggregatedScanResult": "yes"}"#,
    GuardrailStatus::GuardrailFailedToRespond
)]
#[case::number_value(
    r#"{"aggregatedScanResult": 1}"#,
    GuardrailStatus::GuardrailFailedToRespond
)]
fn determine_guardrail_status_reads_the_aggregated_result(
    #[case] text: &str,
    #[case] expected: GuardrailStatus,
) {
    assert_eq!(determine_guardrail_status(&response(text)), expected);
}

#[rstest]
#[case::only_pii(
    r#"{"sensitiveData": {"PII": {"result": true}}, "harmfulContent": {"result": false}}"#,
    true
)]
#[case::data_detector(r#"{"dataDetector": {"SECRET": {"result": true}}}"#, true)]
#[case::another_detector_true(
    r#"{"sensitiveData": {"PII": {"result": true}}, "harmfulContent": {"result": true}}"#,
    false
)]
#[case::no_sensitive_data(
    r#"{"sensitiveData": {"PII": {"result": false}}, "harmfulContent": {"result": false}}"#,
    false
)]
#[case::empty_classification("{}", false)]
#[case::nested_topic_true(
    r#"{"sensitiveData": {"PII": {"result": true}}, "customLlm": {"topic": {"result": true}}}"#,
    false
)]
#[case::anonymized_content_is_no_detector(
    r#"{"sensitiveData": {"PII": {"result": true}}, "anonymizedContent": {"anonymized": "x"}}"#,
    true
)]
#[case::string_result_is_no_failure(r#"{"sensitiveData": {"PII": {"result": "true"}}}"#, false)]
#[case::sensitive_value_not_an_object(r#"{"sensitiveData": true}"#, false)]
#[case::non_object_detector_is_ignored(
    r#"{"sensitiveData": {"PII": {"result": true}}, "harmfulContent": true}"#,
    true
)]
fn only_sensitive_data_detectors_allow_anonymization(
    #[case] classification: &str,
    #[case] expected: bool,
) {
    assert_eq!(only_sensitive_data_failed(&data(classification)), expected);
}

#[rstest]
#[case::user(
    r#"{"scanResult": [{"role": "user", "results": {"anonymizedContent": {"anonymized": "clean"}}}]}"#,
    MessageRole::User,
    Some("clean")
)]
#[case::assistant(
    r#"{"scanResult": [{"role": "assistant", "results": {"anonymizedContent": {"anonymized": "clean"}}}]}"#,
    MessageRole::Assistant,
    Some("clean")
)]
#[case::other_role(
    r#"{"scanResult": [{"role": "assistant", "results": {"anonymizedContent": {"anonymized": "clean"}}}]}"#,
    MessageRole::User,
    None
)]
#[case::missing_results(r#"{"scanResult": [{"role": "user"}]}"#, MessageRole::User, None)]
#[case::empty_anonymized(
    r#"{"scanResult": [{"role": "user", "results": {"anonymizedContent": {"anonymized": ""}}}]}"#,
    MessageRole::User,
    None
)]
#[case::no_scan_result("{}", MessageRole::User, None)]
fn extracted_anonymized_content_comes_from_the_matching_role(
    #[case] text: &str,
    #[case] role: MessageRole,
    #[case] expected: Option<&str>,
) {
    let extracted = extract_anonymized_content(&response(text), role).unwrap();
    assert_eq!(extracted.as_deref(), expected);
}

#[rstest]
#[case::non_list_scan_result(r#"{"scanResult": {}}"#, "scanResult")]
#[case::non_object_item(r#"{"scanResult": [7]}"#, "scanResult")]
#[case::non_object_results(
    r#"{"scanResult": [{"role": "user", "results": 7}]}"#,
    "scanResult.results"
)]
#[case::non_object_anonymized_content(
    r#"{"scanResult": [{"role": "user", "results": {"anonymizedContent": 7}}]}"#,
    "anonymizedContent"
)]
#[case::non_string_anonymized(
    r#"{"scanResult": [{"role": "user", "results": {"anonymizedContent": {"anonymized": 7}}}]}"#,
    "anonymizedContent.anonymized"
)]
fn mismatched_anonymized_content_shapes_name_the_field(
    #[case] text: &str,
    #[case] field: &'static str,
) {
    let error = extract_anonymized_content(&response(text), MessageRole::User).unwrap_err();
    assert!(matches!(error, NomaError::ResponseField { field: found } if found == field));
}

#[rstest]
#[case::safe(r#"{"aggregatedScanResult": false}"#, true)]
#[case::missing_verdict("{}", true)]
#[case::null_verdict(r#"{"aggregatedScanResult": null}"#, true)]
#[case::only_sensitive(
    r#"{"aggregatedScanResult": true, "scanResult": [{"role": "user", "results": {"sensitiveData": {"PII": {"result": true}}}}]}"#,
    true
)]
#[case::other_detector(
    r#"{"aggregatedScanResult": true, "scanResult": [{"role": "user", "results": {"sensitiveData": {"PII": {"result": true}}, "harmfulContent": {"result": true}}}]}"#,
    false
)]
#[case::no_scan_result(r#"{"aggregatedScanResult": true}"#, false)]
#[case::empty_scan_result(r#"{"aggregatedScanResult": true, "scanResult": []}"#, false)]
#[case::non_list_scan_result(r#"{"aggregatedScanResult": true, "scanResult": {}}"#, false)]
#[case::other_role(
    r#"{"aggregatedScanResult": true, "scanResult": [{"role": "assistant", "results": {"sensitiveData": {"PII": {"result": true}}}}]}"#,
    false
)]
#[case::missing_results(
    r#"{"aggregatedScanResult": true, "scanResult": [{"role": "user"}]}"#,
    false
)]
#[case::invalid_verdict(
    r#"{"aggregatedScanResult": "yes", "scanResult": [{"role": "user", "results": {"sensitiveData": {"PII": {"result": true}}}}]}"#,
    true
)]
fn should_anonymize_follows_the_verdict_and_the_detectors(
    #[case] text: &str,
    #[case] expected: bool,
) {
    let settings = anonymizing_settings();
    assert_eq!(
        should_anonymize(&settings, &response(text), MessageRole::User).unwrap(),
        expected
    );
}

#[rstest]
#[case::monitor_mode(monitoring_settings())]
#[case::disabled(settings())]
fn anonymization_never_applies_outside_its_flags(#[case] settings: NomaSettings) {
    let reply = r#"{"aggregatedScanResult": false}"#;
    assert!(!should_anonymize(&settings, &response(reply), MessageRole::User).unwrap());
}

#[derive(Clone, Copy, Debug)]
enum VerdictCall {
    Background,
    Check,
}

#[rstest]
#[case::background_blocked(
    VerdictCall::Background,
    r#"{"aggregatedScanResult": true}"#,
    tracing::Level::WARN,
    "Noma guardrail blocked user message: hi"
)]
#[case::background_allowed(
    VerdictCall::Background,
    r#"{"aggregatedScanResult": false}"#,
    tracing::Level::INFO,
    "Noma guardrail allowed user message: hi"
)]
#[case::check_blocked(
    VerdictCall::Check,
    r#"{"aggregatedScanResult": true}"#,
    tracing::Level::WARN,
    "Noma guardrail blocked user message: hi"
)]
#[case::check_allowed(
    VerdictCall::Check,
    r#"{"aggregatedScanResult": false}"#,
    tracing::Level::INFO,
    "Noma guardrail allowed user message: hi"
)]
fn monitor_verdicts_are_logged(
    #[case] call: VerdictCall,
    #[case] reply: &str,
    #[case] level: tracing::Level,
    #[case] expected: &str,
) {
    let settings = monitoring_settings();
    let captured_log = CapturedLog::default();
    with_default(captured_log.subscriber(level), || match call {
        VerdictCall::Background => {
            handle_verdict_background(MessageRole::User, "hi", &response(reply));
        }
        VerdictCall::Check => {
            check_verdict(&settings, MessageRole::User, "hi", &response(reply)).unwrap();
        }
    });
    assert!(captured_log.text().contains(expected));
}
