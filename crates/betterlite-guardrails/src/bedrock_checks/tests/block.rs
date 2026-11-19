#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_bedrock_guardrails::bedrock_transport::BedrockGuardrail;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde_json::Number;

use super::support::{
    GUARDRAIL_NAME, Label, as_array, as_object, base, built, checks_named, field, guardrail,
    guardrail_init, scored, truncated_violation,
};
use crate::bedrock_checks::block::exception;
use crate::bedrock_records::ChecksViolation;
use crate::guardrail_base::failure::GuardrailFailure;

fn blocking_guardrail() -> BedrockGuardrail {
    let mut init = guardrail_init(checks_named(&["contentFilter"]));
    init.disable_exception_on_block = Some(true);
    built(init)
}

fn request_data() -> BTreeMap<String, GuardrailValue> {
    BTreeMap::from([(
        "model".to_owned(),
        GuardrailValue::Text("claude-3".to_owned()),
    )])
}

#[test]
fn http_exception_on_violations() {
    let found = vec![scored("contentFilter", Label::Category("VIOLENCE"), 0.8)];
    let failure = exception(&guardrail(), &base(), &found, &request_data());
    let GuardrailFailure::HttpObject { status, detail } = failure else {
        panic!("expected a 400 http object: {failure:?}");
    };
    assert_eq!(status, 400);
    assert_eq!(
        detail.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["error", "bedrock_guardrail_checks"]
    );
    assert_eq!(
        detail.get("error"),
        Some(&GuardrailValue::Text(
            "Violated guardrail policy".to_owned()
        ))
    );
    let checks = as_array(
        detail
            .get("bedrock_guardrail_checks")
            .expect("the check list"),
    );
    assert_eq!(checks.len(), 1);
    let violation = as_object(&checks[0]);
    assert_eq!(
        field(violation, "check"),
        GuardrailValue::Text("contentFilter".to_owned())
    );
    assert_eq!(
        field(violation, "category"),
        GuardrailValue::Text("VIOLENCE".to_owned())
    );
    assert_eq!(
        field(violation, "severityScore"),
        GuardrailValue::Decimal(0.8)
    );
}

#[test]
fn modify_response_on_violations() {
    let found = vec![scored("contentFilter", Label::Category("VIOLENCE"), 0.8)];
    let failure = exception(&blocking_guardrail(), &base(), &found, &request_data());
    let GuardrailFailure::ModifyResponse(response) = failure else {
        panic!("expected a modify response: {failure:?}");
    };
    assert_eq!(response.message, "Violated guardrail policy");
    assert_eq!(response.model, "claude-3");
    assert_eq!(response.guardrail_name.as_deref(), Some(GUARDRAIL_NAME));
    assert!(response.detection_info.is_empty());
    assert_eq!(response.original_response, None);
    assert_eq!(response.request_data, request_data());
}

#[test]
fn modify_response_uses_the_default_model_when_absent() {
    let found = vec![scored("contentFilter", Label::Category("VIOLENCE"), 0.8)];
    let failure = exception(&blocking_guardrail(), &base(), &found, &BTreeMap::new());
    let GuardrailFailure::ModifyResponse(response) = failure else {
        panic!("expected a modify response: {failure:?}");
    };
    assert_eq!(response.model, "bedrock-guardrail");
    let null_model = BTreeMap::from([("model".to_owned(), GuardrailValue::Null)]);
    let null_failure = exception(&blocking_guardrail(), &base(), &found, &null_model);
    let GuardrailFailure::ModifyResponse(null_response) = null_failure else {
        panic!("expected a modify response: {null_failure:?}");
    };
    assert_eq!(null_response.model, "bedrock-guardrail");
}

#[test]
fn block_detail_writes_a_missing_label_as_null() {
    let found = vec![ChecksViolation {
        check: Some("sensitiveInformation".to_owned()),
        category: None,
        type_name: None,
        severity_score: None,
        confidence_score: Number::from_f64(0.8),
        truncated: None,
        extra: BTreeMap::new(),
    }];
    let failure = exception(&guardrail(), &base(), &found, &request_data());
    let GuardrailFailure::HttpObject { detail, .. } = failure else {
        panic!("expected a 400 http object: {failure:?}");
    };
    let checks = as_array(
        detail
            .get("bedrock_guardrail_checks")
            .expect("the check list"),
    );
    let violation = as_object(&checks[0]);
    assert_eq!(field(violation, "type"), GuardrailValue::Null);
    assert_eq!(
        field(violation, "confidenceScore"),
        GuardrailValue::Decimal(0.8)
    );
    assert!(!violation.contains_key("category"));
    assert!(!violation.contains_key("severityScore"));
}

#[test]
fn block_detail_writes_the_truncated_violation() {
    let found = vec![truncated_violation()];
    let failure = exception(&guardrail(), &base(), &found, &request_data());
    let GuardrailFailure::HttpObject { detail, .. } = failure else {
        panic!("expected a 400 http object: {failure:?}");
    };
    let checks = as_array(
        detail
            .get("bedrock_guardrail_checks")
            .expect("the check list"),
    );
    assert_eq!(
        *as_object(&checks[0]),
        BTreeMap::from([
            (
                "check".to_owned(),
                GuardrailValue::Text("sensitiveInformation".to_owned())
            ),
            ("truncated".to_owned(), GuardrailValue::Boolean(true)),
        ])
    );
}

#[test]
fn block_detail_keeps_an_integral_score() {
    let found = vec![scored("contentFilter", Label::Category("VIOLENCE"), 1.0)];
    let failure = exception(&guardrail(), &base(), &found, &request_data());
    let GuardrailFailure::HttpObject { detail, .. } = failure else {
        panic!("expected a 400 http object: {failure:?}");
    };
    let checks = as_array(
        detail
            .get("bedrock_guardrail_checks")
            .expect("the check list"),
    );
    assert_eq!(
        field(as_object(&checks[0]), "severityScore"),
        GuardrailValue::Decimal(1.0)
    );
}
