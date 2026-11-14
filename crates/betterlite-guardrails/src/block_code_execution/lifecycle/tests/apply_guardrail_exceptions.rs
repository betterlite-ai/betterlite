use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::block_code_execution::registration::{
    ApplyGuardrailError, apply_guardrail, status_for_failure,
};
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::failure::{GuardrailFailure, ModifyResponseFailure};
use betterlite_guardrails::guardrail_records::record::{GuardrailRecordContext, GuardrailStatus};
use indexmap::IndexMap;

use crate::block_code_execution::support::{
    FailingSink, apply_capturing, guardrail, inputs, params, request_data,
};

fn passthrough() -> GuardrailFailure {
    GuardrailFailure::ModifyResponse(Box::new(ModifyResponseFailure {
        message: "blocked".to_owned(),
        model: "gpt-4o".to_owned(),
        request_data: BTreeMap::new(),
        guardrail_name: Some("block_code_execution".to_owned()),
        detection_info: BTreeMap::new(),
        original_response: None,
    }))
}

#[test]
fn a_passthrough_block_is_an_intervention() {
    assert_eq!(
        status_for_failure(&passthrough()),
        GuardrailStatus::GuardrailIntervened
    );
}

#[test]
fn an_http_400_is_an_intervention() {
    let failure = GuardrailFailure::HttpObject {
        status: 400,
        detail: IndexMap::new(),
    };
    assert_eq!(
        status_for_failure(&failure),
        GuardrailStatus::GuardrailIntervened
    );
}

#[test]
fn any_other_failure_is_a_failed_response() {
    let failure = GuardrailFailure::Other("the scanner exploded".to_owned());
    assert_eq!(
        status_for_failure(&failure),
        GuardrailStatus::GuardrailFailedToRespond
    );
    let provider = GuardrailFailure::Http {
        status: 500,
        detail: "upstream".to_owned(),
    };
    assert_eq!(
        status_for_failure(&provider),
        GuardrailStatus::GuardrailFailedToRespond
    );
}

#[test]
fn a_request_block_records_an_intervention() {
    let built = guardrail(params());
    let mut data = request_data();
    let (outcome, records) = apply_capturing(
        &built,
        inputs(&["run this ```python\nprint(1)\n```"]),
        &mut data,
        GuardrailInputType::Request,
    );
    assert!(matches!(outcome, Err(_)));
    assert_eq!(
        records[0].guardrail_status,
        GuardrailStatus::GuardrailIntervened
    );
}

#[test]
fn the_block_language_comes_from_the_first_blocked_detection() {
    let mut configured = params();
    configured.blocked_languages = Some(vec!["python".to_owned()]);
    let built = guardrail(configured);
    let mut data = request_data();
    let text = "```rust\na\n```\n```python\nb\n```";
    let (outcome, records) = apply_capturing(
        &built,
        inputs(&[text]),
        &mut data,
        GuardrailInputType::Response,
    );
    let Err(ApplyGuardrailError::Guardrail(GuardrailFailure::HttpObject { detail, .. })) = outcome
    else {
        panic!("a response block must be an object failure");
    };
    assert_eq!(
        detail.get("language"),
        Some(&GuardrailValue::Text("python".to_owned()))
    );
    let GuardrailValue::Array(detections) = &records[0].guardrail_response else {
        panic!("the record carries its detections");
    };
    assert_eq!(detections.len(), 2);
    let GuardrailValue::Object(first) = &detections[0] else {
        panic!("a detection is an object");
    };
    assert_eq!(
        first.get("language"),
        Some(&GuardrailValue::Text("rust".to_owned()))
    );
    assert_eq!(
        first.get("action_taken"),
        Some(&GuardrailValue::Text("allow".to_owned()))
    );
}

#[test]
fn a_response_block_records_an_intervention() {
    let built = guardrail(params());
    let mut data = request_data();
    let (outcome, records) = apply_capturing(
        &built,
        inputs(&["```python\nprint(1)\n```"]),
        &mut data,
        GuardrailInputType::Response,
    );
    assert!(matches!(outcome, Err(_)));
    assert_eq!(
        records[0].guardrail_status,
        GuardrailStatus::GuardrailIntervened
    );
}

#[test]
fn a_successful_scan_records_success() {
    let built = guardrail(params());
    let mut data = request_data();
    let (outcome, records) = apply_capturing(
        &built,
        inputs(&["no code here"]),
        &mut data,
        GuardrailInputType::Request,
    );
    assert!(outcome.is_ok());
    assert_eq!(records[0].guardrail_status, GuardrailStatus::Success);
}

#[test]
fn a_failing_span_sink_does_not_change_the_outcome() {
    let built = guardrail(params());
    let mut data = request_data();
    let mut context = GuardrailRecordContext {
        base: &built.base,
        request_data: &mut data,
        log_target: None,
        sink: &FailingSink,
    };
    let outcome = apply_guardrail(
        &built,
        inputs(&["plain text"]),
        &mut context,
        GuardrailInputType::Request,
    );
    assert!(outcome.is_ok());
}

#[test]
fn the_record_keeps_the_guardrail_name_and_provider() {
    let built = guardrail(params());
    let mut data = request_data();
    let (_outcome, records) = apply_capturing(
        &built,
        inputs(&["plain text"]),
        &mut data,
        GuardrailInputType::Request,
    );
    let record = &records[0];
    assert_eq!(
        record.guardrail_name.as_deref(),
        Some("block_code_execution")
    );
    assert_eq!(
        record.guardrail_provider.as_deref(),
        Some("block_code_execution")
    );
    let GuardrailValue::Array(detections) = &record.guardrail_response else {
        panic!("a successful record carries a list");
    };
    assert!(detections.is_empty());
}
