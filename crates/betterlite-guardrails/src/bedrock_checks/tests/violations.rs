#![cfg(test)]

use std::collections::BTreeMap;

use super::support::{
    Label, checks_named, checks_response, scored, thresholds, truncated_violation,
};
use crate::bedrock_checks::violations::{collect, status, tracing_detail};
use crate::bedrock_records::ChecksViolation;
use crate::guardrail_records::record::GuardrailStatus;
use serde_json::Number;

#[test]
fn content_filter_at_the_threshold_is_a_violation() {
    let response = checks_response(
        r#"{"results":{"contentFilter":{"results":[{"category":"VIOLENCE","severityScore":0.8}]}}}"#,
    );
    let found = collect(
        Some(&response),
        &checks_named(&["contentFilter"]),
        thresholds(Some(0.8_f64), None, None),
    );
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].check.as_deref(), Some("contentFilter"));
    assert_eq!(found[0].category.as_deref(), Some("VIOLENCE"));
    assert_eq!(found[0].severity_score, Number::from_f64(0.8));
    assert_eq!(found[0].confidence_score, None);
}

#[test]
fn content_filter_below_the_threshold_is_not_a_violation() {
    let response = checks_response(
        r#"{"results":{"contentFilter":{"results":[{"category":"VIOLENCE","severityScore":0.6}]}}}"#,
    );
    let found = collect(
        Some(&response),
        &checks_named(&["contentFilter"]),
        thresholds(Some(0.8_f64), None, None),
    );
    assert!(found.is_empty());
}

#[test]
fn threshold_none_makes_a_check_detect_only() {
    let response = checks_response(
        r#"{"results":{"contentFilter":{"results":[{"category":"VIOLENCE","severityScore":1.0}]}}}"#,
    );
    let found = collect(
        Some(&response),
        &checks_named(&["contentFilter"]),
        thresholds(None, None, None),
    );
    assert!(found.is_empty());
}

#[test]
fn unconfigured_check_is_not_evaluated() {
    let response = checks_response(
        r#"{"results":{"contentFilter":{"results":[{"category":"VIOLENCE","severityScore":1.0}]}}}"#,
    );
    let found = collect(
        Some(&response),
        &checks_named(&["promptAttack"]),
        thresholds(Some(0.5_f64), Some(0.5_f64), None),
    );
    assert!(found.is_empty());
}

#[test]
fn results_of_an_unconfigured_check_are_not_evaluated() {
    let response = checks_response(
        r#"{"results":{"promptAttack":{"results":[{"category":"PROMPT_INJECTION","severityScore":1.0}]},"sensitiveInformation":{"results":[{"type":"EMAIL","confidenceScore":0.9}]}}}"#,
    );
    let found = collect(
        Some(&response),
        &checks_named(&["contentFilter"]),
        thresholds(None, Some(0.5_f64), Some(0.5_f64)),
    );
    assert!(found.is_empty());
}

#[test]
fn prompt_attack_threshold_match_is_a_violation() {
    let response = checks_response(
        r#"{"results":{"promptAttack":{"results":[{"category":"PROMPT_INJECTION","severityScore":1.0}]}}}"#,
    );
    let found = collect(
        Some(&response),
        &checks_named(&["promptAttack"]),
        thresholds(None, Some(0.5_f64), None),
    );
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].check.as_deref(), Some("promptAttack"));
    assert_eq!(found[0].category.as_deref(), Some("PROMPT_INJECTION"));
    assert_eq!(found[0].severity_score, Number::from_f64(1.0));
}

#[test]
fn pii_confidence_at_the_threshold_is_a_violation() {
    let response = checks_response(
        r#"{"results":{"sensitiveInformation":{"results":[{"type":"EMAIL","confidenceScore":0.9}]}}}"#,
    );
    let found = collect(
        Some(&response),
        &checks_named(&["sensitiveInformation"]),
        thresholds(None, None, Some(0.9_f64)),
    );
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].check.as_deref(), Some("sensitiveInformation"));
    assert_eq!(found[0].type_name.as_deref(), Some("EMAIL"));
    assert_eq!(found[0].confidence_score, Number::from_f64(0.9));
    assert_eq!(found[0].severity_score, None);
}

#[test]
fn pii_below_the_threshold_is_not_a_violation() {
    let response = checks_response(
        r#"{"results":{"sensitiveInformation":{"results":[{"type":"EMAIL","confidenceScore":0.5}]}}}"#,
    );
    let found = collect(
        Some(&response),
        &checks_named(&["sensitiveInformation"]),
        thresholds(None, None, Some(0.9_f64)),
    );
    assert!(found.is_empty());
}

#[test]
fn truncated_sensitive_information_is_a_violation_without_entries() {
    let response = checks_response(r#"{"results":{"sensitiveInformation":{"truncated":true}}}"#);
    let found = collect(
        Some(&response),
        &checks_named(&["sensitiveInformation"]),
        thresholds(None, None, Some(0.9_f64)),
    );
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].truncated, Some(true));
    assert_eq!(found[0].check.as_deref(), Some("sensitiveInformation"));
    assert_eq!(found[0].type_name, None);
    assert_eq!(found[0].confidence_score, None);
}

#[test]
fn truncated_sensitive_information_without_a_threshold_is_ignored() {
    let response = checks_response(r#"{"results":{"sensitiveInformation":{"truncated":true}}}"#);
    let found = collect(
        Some(&response),
        &checks_named(&["sensitiveInformation"]),
        thresholds(None, None, None),
    );
    assert!(found.is_empty());
}

#[test]
fn truncated_false_adds_no_violation_of_its_own() {
    let response = checks_response(
        r#"{"results":{"sensitiveInformation":{"truncated":false,"results":[{"type":"EMAIL","confidenceScore":0.9}]}}}"#,
    );
    let found = collect(
        Some(&response),
        &checks_named(&["sensitiveInformation"]),
        thresholds(None, None, Some(0.9_f64)),
    );
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].truncated, None);
    assert_eq!(found[0].type_name.as_deref(), Some("EMAIL"));
}

#[test]
fn truncated_result_counts_before_its_scored_entries() {
    let response = checks_response(
        r#"{"results":{"sensitiveInformation":{"truncated":true,"results":[{"type":"EMAIL","confidenceScore":0.9},{"type":"PHONE","confidenceScore":0.5}]}}}"#,
    );
    let found = collect(
        Some(&response),
        &checks_named(&["sensitiveInformation"]),
        thresholds(None, None, Some(0.9_f64)),
    );
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].truncated, Some(true));
    assert_eq!(found[1].type_name.as_deref(), Some("EMAIL"));
}

#[test]
fn entry_without_a_score_is_not_a_violation() {
    let content =
        checks_response(r#"{"results":{"contentFilter":{"results":[{"category":"VIOLENCE"}]}}}"#);
    let found = collect(
        Some(&content),
        &checks_named(&["contentFilter"]),
        thresholds(Some(0.5_f64), None, None),
    );
    assert!(found.is_empty());
    let pii =
        checks_response(r#"{"results":{"sensitiveInformation":{"results":[{"type":"EMAIL"}]}}}"#);
    let pii_found = collect(
        Some(&pii),
        &checks_named(&["sensitiveInformation"]),
        thresholds(None, None, Some(0.5_f64)),
    );
    assert!(pii_found.is_empty());
}

#[test]
fn a_response_without_results_yields_no_violations() {
    let response = checks_response(r#"{"usage":{"contentFilter":{"textUnits":3}}}"#);
    let found = collect(
        Some(&response),
        &checks_named(&["contentFilter", "promptAttack", "sensitiveInformation"]),
        thresholds(Some(0.0_f64), Some(0.0_f64), Some(0.0_f64)),
    );
    assert!(found.is_empty());
    let without_response = collect(
        None,
        &checks_named(&["contentFilter"]),
        thresholds(Some(0.0_f64), None, None),
    );
    assert!(without_response.is_empty());
}

#[test]
fn status_is_intervened_only_over_the_threshold() {
    assert_eq!(status(true), GuardrailStatus::GuardrailIntervened);
    assert_eq!(status(false), GuardrailStatus::Success);
}

#[test]
fn tracing_detail_collects_categories_and_the_action() {
    let found = vec![
        scored("contentFilter", &Label::Category("VIOLENCE"), 0.9),
        scored("sensitiveInformation", &Label::TypeName("EMAIL"), 0.8),
    ];
    let detail = tracing_detail(&found);
    assert_eq!(
        detail.violation_categories,
        Some(vec!["VIOLENCE".to_owned(), "EMAIL".to_owned()])
    );
    assert_eq!(
        detail.guardrail_action.as_deref(),
        Some("GUARDRAIL_INTERVENED")
    );
}

#[test]
fn tracing_detail_falls_back_to_the_type_and_skips_empty_labels() {
    let found = vec![
        ChecksViolation {
            check: Some("contentFilter".to_owned()),
            category: Some(String::new()),
            type_name: Some("UNLABELLED".to_owned()),
            severity_score: Number::from_f64(0.9),
            confidence_score: None,
            truncated: None,
            extra: BTreeMap::new(),
        },
        scored("contentFilter", &Label::Category(""), 0.9),
    ];
    let detail = tracing_detail(&found);
    assert_eq!(
        detail.violation_categories,
        Some(vec!["UNLABELLED".to_owned()])
    );
}

#[test]
fn tracing_detail_without_labels_has_no_categories() {
    let found = vec![truncated_violation()];
    let detail = tracing_detail(&found);
    assert_eq!(detail.violation_categories, None);
    assert_eq!(
        detail.guardrail_action.as_deref(),
        Some("GUARDRAIL_INTERVENED")
    );
}

#[test]
fn tracing_detail_without_violations_has_the_none_action() {
    let detail = tracing_detail(&[]);
    assert_eq!(detail.violation_categories, None);
    assert_eq!(detail.guardrail_action.as_deref(), Some("NONE"));
}
