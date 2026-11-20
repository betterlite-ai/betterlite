#![cfg(test)]

use super::support::checks_response;
use crate::bedrock_checks::sanitize::for_logging;
use crate::bedrock_records::{ChecksPiiEntry, ChecksResponse};
use serde_json::Number;

const LOCATION_KEYS: [&str; 4] = ["messageIndex", "contentIndex", "beginOffset", "endOffset"];

fn pii_entry(response: &ChecksResponse) -> &ChecksPiiEntry {
    response
        .results
        .as_ref()
        .and_then(|results| results.sensitive_information.as_ref())
        .and_then(|sensitive| sensitive.results.as_ref())
        .and_then(|entries| entries.first())
        .expect("the PII entry")
}

fn as_json(response: &ChecksResponse) -> String {
    serde_json::to_string(response).expect("the checks response as JSON")
}

#[test]
fn logging_copy_strips_every_pii_location_offset() {
    let response = checks_response(
        r#"{"results":{"sensitiveInformation":{"results":[{"type":"EMAIL","confidenceScore":0.9,"messageIndex":1,"contentIndex":0,"beginOffset":5,"endOffset":17}]}}}"#,
    );
    let sanitized = for_logging(&response);
    let entry = pii_entry(&sanitized);
    assert_eq!(entry.message_index, None);
    assert_eq!(entry.content_index, None);
    assert_eq!(entry.begin_offset, None);
    assert_eq!(entry.end_offset, None);
    assert_eq!(entry.type_name.as_deref(), Some("EMAIL"));
    assert_eq!(entry.confidence_score, Number::from_f64(0.9));
    let json = as_json(&sanitized);
    assert!(LOCATION_KEYS.iter().all(|key| !json.contains(key)));
}

#[test]
fn logging_copy_keeps_the_original_response_untouched() {
    let response = checks_response(
        r#"{"results":{"sensitiveInformation":{"results":[{"type":"EMAIL","confidenceScore":0.9,"messageIndex":1,"beginOffset":5}]}}}"#,
    );
    let sanitized = for_logging(&response);
    assert!(!as_json(&sanitized).contains("messageIndex"));
    let entry = pii_entry(&response);
    assert_eq!(entry.message_index, Some(1));
    assert_eq!(entry.begin_offset, Some(5));
}

#[test]
fn logging_copy_strips_location_keys_held_as_extras() {
    let response = checks_response(
        r#"{"results":{"sensitiveInformation":{"results":[{"type":"EMAIL","confidenceScore":0.9,"beginOffset":null,"endOffset":null}]}}}"#,
    );
    let entry = pii_entry(&response);
    assert!(entry.extra.contains_key("beginOffset"));
    assert!(entry.extra.contains_key("endOffset"));
    let sanitized = for_logging(&response);
    let sanitized_entry = pii_entry(&sanitized);
    assert!(sanitized_entry.extra.is_empty());
    let json = as_json(&sanitized);
    assert!(LOCATION_KEYS.iter().all(|key| !json.contains(key)));
}

#[test]
fn logging_copy_writes_integral_scores_as_floats() {
    let response = checks_response(
        r#"{"results":{"contentFilter":{"results":[{"category":"VIOLENCE","severityScore":1}]},"promptAttack":{"results":[{"category":"PROMPT_INJECTION","severityScore":0}]},"sensitiveInformation":{"results":[{"type":"EMAIL","confidenceScore":1}]}}}"#,
    );
    let json = as_json(&for_logging(&response));
    assert!(json.contains(r#""severityScore":1.0"#));
    assert!(json.contains(r#""severityScore":0.0"#));
    assert!(json.contains(r#""confidenceScore":1.0"#));
}

#[test]
fn logging_copy_keeps_a_fractional_score_unchanged() {
    let response = checks_response(
        r#"{"results":{"sensitiveInformation":{"results":[{"type":"EMAIL","confidenceScore":0.85}]}}}"#,
    );
    let sanitized = for_logging(&response);
    let entry = pii_entry(&sanitized);
    assert_eq!(entry.confidence_score, Number::from_f64(0.85));
}

#[test]
fn logging_copy_keeps_an_absent_score_absent() {
    let response =
        checks_response(r#"{"results":{"sensitiveInformation":{"results":[{"type":"EMAIL"}]}}}"#);
    let sanitized = for_logging(&response);
    let entry = pii_entry(&sanitized);
    assert_eq!(entry.confidence_score, None);
    assert!(!as_json(&sanitized).contains("confidenceScore"));
}

#[test]
fn logging_copy_keeps_the_truncated_flag() {
    let response = checks_response(
        r#"{"results":{"sensitiveInformation":{"truncated":true,"results":[{"type":"EMAIL","confidenceScore":0.9}]}}}"#,
    );
    let sanitized = for_logging(&response);
    let sensitive = sanitized
        .results
        .as_ref()
        .and_then(|results| results.sensitive_information.as_ref())
        .expect("the sensitive information result");
    assert_eq!(sensitive.truncated, Some(true));
    assert!(as_json(&sanitized).contains(r#""truncated":true"#));
}

#[test]
fn logging_copy_of_an_empty_response_stays_empty() {
    let response = checks_response("{}");
    assert_eq!(as_json(&for_logging(&response)), "{}");
}

#[test]
fn logging_copy_without_sensitive_results_leaves_the_other_checks() {
    let response = checks_response(
        r#"{"results":{"contentFilter":{"results":[{"category":"VIOLENCE","severityScore":0.4}]}}}"#,
    );
    let json = as_json(&for_logging(&response));
    assert_eq!(
        json,
        r#"{"results":{"contentFilter":{"results":[{"category":"VIOLENCE","severityScore":0.4}]}}}"#
    );
}
