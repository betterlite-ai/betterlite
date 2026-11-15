#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Ready, ready};
use std::sync::{Arc, Mutex};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;
use tokio::runtime::Runtime;

use super::error::{ConductCheckError, ConductError};
use super::{
    BLOCKING_VERDICTS, ConductCheck, ConductDecision, ConductEnforcement, FLAGGED_VERDICTS,
    apply_conduct_guardrail, binds_unreachable_fallback, decision_status, record_decision,
    request_payload,
};
use crate::generic_guardrail_contracts::{
    GuardrailInputType, GuardrailMessage, GuardrailToolParam,
};
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::guardrail_base::settings::GuardrailSettings;
use crate::guardrail_records::error::{GuardrailRecordError, GuardrailSpanError};
use crate::guardrail_records::record::{
    GuardrailInvocationRecord, GuardrailRecordContext, GuardrailSpanSink, GuardrailStatus,
};

struct DiscardSink;

impl GuardrailSpanSink for DiscardSink {
    fn emit(&self, _record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        Ok(())
    }
}

struct FixedCheck {
    decision: ConductDecision,
    calls: Arc<Mutex<Vec<String>>>,
}

impl ConductCheck for FixedCheck {
    fn call(
        &self,
        data: &BTreeMap<String, GuardrailValue>,
        call_type: &str,
    ) -> Ready<Result<ConductDecision, ConductError>> {
        self.calls
            .lock()
            .expect("the call log is not poisoned")
            .push(format!("{call_type}:{}", data.len()));
        ready(Ok(self.decision.clone()))
    }
}

fn decision(verdict: &str, rule_id: Option<&str>) -> ConductDecision {
    ConductDecision {
        verdict: verdict.to_owned(),
        rule_id: rule_id.map(str::to_owned),
    }
}

fn recorded_failure(decision: &ConductDecision) -> GuardrailFailure {
    GuardrailFailure::Http {
        status: 400,
        detail: decision.verdict.clone(),
    }
}

fn record_context<'state>(
    request_data: &'state mut BTreeMap<String, GuardrailValue>,
    base: &'state GuardrailBase,
    sink: &'state DiscardSink,
) -> GuardrailRecordContext<'state> {
    GuardrailRecordContext {
        base,
        request_data,
        log_target: None,
        sink,
    }
}

fn named_base() -> GuardrailBase {
    let mut base = GuardrailBase {
        settings: GuardrailSettings::default(),
        mode: None,
    };
    base.settings.guardrail_name = Some("conduct".to_owned());
    base
}

fn user_message(content: &str) -> GuardrailMessage {
    GuardrailMessage {
        role: "user".to_owned(),
        extra: BTreeMap::from([(
            "content".to_owned(),
            GuardrailValue::Text(content.to_owned()),
        )]),
    }
}

fn text_inputs(values: &[&str]) -> GuardrailApiInputs {
    GuardrailApiInputs {
        texts: Some(values.iter().map(|text| (*text).to_owned()).collect()),
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    }
}

fn structured_inputs(messages: Vec<GuardrailMessage>) -> GuardrailApiInputs {
    GuardrailApiInputs {
        texts: None,
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: Some(messages),
        model: None,
        stream_holdback_chars: None,
    }
}

#[test]
fn conduct_guardrail_request_payload_builds_user_messages_from_texts() {
    let inputs = text_inputs(&["first", "second"]);
    let request_data =
        BTreeMap::from([("model".to_owned(), GuardrailValue::Text("gpt".to_owned()))]);
    let payload = request_payload(&inputs, &request_data, GuardrailInputType::Request)
        .expect("a request input has a payload");
    assert_eq!(
        payload.get("prompt"),
        Some(&GuardrailValue::Null),
        "the prompt is always null"
    );
    assert_eq!(
        payload.get("model"),
        Some(&GuardrailValue::Text("gpt".to_owned()))
    );
    let messages = payload
        .get("messages")
        .expect("messages are in the payload");
    assert_eq!(
        messages,
        &GuardrailValue::Array(vec![
            GuardrailValue::Object(BTreeMap::from([
                (
                    "content".to_owned(),
                    GuardrailValue::Text("first".to_owned())
                ),
                ("role".to_owned(), GuardrailValue::Text("user".to_owned())),
            ])),
            GuardrailValue::Object(BTreeMap::from([
                (
                    "content".to_owned(),
                    GuardrailValue::Text("second".to_owned())
                ),
                ("role".to_owned(), GuardrailValue::Text("user".to_owned())),
            ])),
        ])
    );
}

#[test]
fn conduct_guardrail_request_payload_prefers_structured_messages() {
    let mut inputs = structured_inputs(vec![user_message("structured")]);
    inputs.texts = Some(vec!["ignored".to_owned()]);
    let payload = request_payload(&inputs, &BTreeMap::new(), GuardrailInputType::Request)
        .expect("a request input has a payload");
    assert_eq!(
        payload.get("messages"),
        Some(&GuardrailValue::Array(vec![GuardrailValue::Object(
            BTreeMap::from([
                (
                    "content".to_owned(),
                    GuardrailValue::Text("structured".to_owned())
                ),
                ("role".to_owned(), GuardrailValue::Text("user".to_owned())),
            ])
        )]))
    );
}

#[test]
fn conduct_guardrail_request_payload_falls_back_to_texts_for_empty_structured_messages() {
    let mut inputs = structured_inputs(Vec::new());
    inputs.texts = Some(vec!["from texts".to_owned()]);
    let payload = request_payload(&inputs, &BTreeMap::new(), GuardrailInputType::Request)
        .expect("a request input has a payload");
    let messages = payload
        .get("messages")
        .expect("messages are in the payload");
    assert_eq!(
        messages,
        &GuardrailValue::Array(vec![GuardrailValue::Object(BTreeMap::from([
            (
                "content".to_owned(),
                GuardrailValue::Text("from texts".to_owned())
            ),
            ("role".to_owned(), GuardrailValue::Text("user".to_owned())),
        ]))])
    );
}

#[test]
fn conduct_guardrail_request_payload_builds_an_empty_message_list_without_inputs() {
    let payload = request_payload(
        &GuardrailApiInputs::default(),
        &BTreeMap::new(),
        GuardrailInputType::Request,
    )
    .expect("a request input has a payload");
    assert_eq!(
        payload.get("messages"),
        Some(&GuardrailValue::Array(Vec::new()))
    );
}

#[test]
fn conduct_guardrail_request_payload_is_none_for_a_response() {
    let inputs = text_inputs(&["response text"]);
    assert_eq!(
        request_payload(&inputs, &BTreeMap::new(), GuardrailInputType::Response),
        None
    );
}

#[test]
fn conduct_guardrail_request_payload_keeps_every_request_data_field() {
    let inputs = GuardrailApiInputs {
        texts: None,
        images: None,
        tools: Some(vec![GuardrailToolParam {
            tool_type: "function".to_owned(),
            extra: BTreeMap::new(),
        }]),
        tool_calls: None,
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    };
    let request_data = BTreeMap::from([
        (
            "messages".to_owned(),
            GuardrailValue::Text("original".to_owned()),
        ),
        ("metadata".to_owned(), GuardrailValue::Null),
    ]);
    let payload = request_payload(&inputs, &request_data, GuardrailInputType::Request)
        .expect("a request input has a payload");
    assert_eq!(payload.len(), 3);
    assert_eq!(
        payload.get("metadata"),
        Some(&GuardrailValue::Null),
        "a copied field keeps its null value"
    );
    assert_eq!(
        payload.get("messages"),
        Some(&GuardrailValue::Array(Vec::new())),
        "the built messages replace the copied key"
    );
}

#[rstest]
#[case::warning("warning")]
#[case::advisory("advisory")]
fn conduct_guardrail_decision_status_flags_a_warning_or_advisory(#[case] verdict: &str) {
    assert_eq!(
        decision_status(&decision(verdict, None)),
        GuardrailStatus::GuardrailFlagged
    );
}

#[rstest]
#[case::block("block")]
#[case::approval("approval")]
#[case::allow("allow")]
#[case::unknown("other")]
fn conduct_guardrail_decision_status_is_success_for_other_verdicts(#[case] verdict: &str) {
    assert_eq!(
        decision_status(&decision(verdict, None)),
        GuardrailStatus::Success
    );
}

#[test]
fn conduct_guardrail_record_decision_writes_the_verdict_and_status() {
    let base = named_base();
    let sink = DiscardSink;
    let mut request_data = BTreeMap::new();
    let mut context = record_context(&mut request_data, &base, &sink);
    record_decision(&mut context, &decision("warning", Some("rule-1")))
        .expect("the record is written");
    let entry = serde_json::to_value(&request_data)
        .expect("the request data is JSON")
        .pointer("/metadata/standard_logging_guardrail_information/0")
        .cloned()
        .expect("one record");
    assert_eq!(entry["guardrail_response"]["verdict"], "warning");
    assert_eq!(entry["guardrail_response"]["rule_id"], "rule-1");
    assert_eq!(entry["guardrail_status"], "guardrail_flagged");
    assert_eq!(entry["guardrail_name"], "conduct");
}

#[test]
fn conduct_guardrail_record_decision_drops_a_null_rule_id() {
    let base = named_base();
    let sink = DiscardSink;
    let mut request_data = BTreeMap::new();
    let mut context = record_context(&mut request_data, &base, &sink);
    record_decision(&mut context, &decision("block", None)).expect("the record is written");
    let entry = serde_json::to_value(&request_data)
        .expect("the request data is JSON")
        .pointer("/metadata/standard_logging_guardrail_information/0")
        .cloned()
        .expect("one record");
    assert_eq!(entry["guardrail_response"]["verdict"], "block");
    assert_eq!(entry["guardrail_response"].get("rule_id"), None);
    assert_eq!(entry["guardrail_status"], "success");
}

#[rstest]
#[case::block("block")]
#[case::approval("approval")]
fn conduct_guardrail_apply_raises_the_blocked_failure(#[case] verdict: &str) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let check = FixedCheck {
        decision: decision(verdict, None),
        calls: Arc::clone(&calls),
    };
    let blocked: fn(&ConductDecision) -> GuardrailFailure = recorded_failure;
    let mut recorded = Vec::new();
    let mut record = |item: &ConductDecision| -> Result<(), GuardrailRecordError> {
        recorded.push(item.clone());
        Ok(())
    };
    let mut request_data = BTreeMap::new();
    let mut enforcement = ConductEnforcement {
        request_data: &mut request_data,
        blocked,
        record: &mut record,
    };
    let inputs = text_inputs(&["hello"]);
    let error = Runtime::new()
        .expect("a runtime")
        .block_on(apply_conduct_guardrail(
            inputs,
            GuardrailInputType::Request,
            &check,
            &mut enforcement,
        ))
        .expect_err("a blocking verdict fails the request");
    assert!(matches!(
        error,
        ConductError::Blocked(GuardrailFailure::Http { status: 400, .. })
    ));
    assert!(recorded.is_empty(), "a blocked decision is not recorded");
    assert_eq!(
        *calls.lock().expect("the call log"),
        vec!["request:2".to_owned()]
    );
}

#[test]
fn conduct_guardrail_apply_records_an_allowed_decision_and_returns_the_inputs() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let check = FixedCheck {
        decision: decision("allow", Some("rule-2")),
        calls: Arc::clone(&calls),
    };
    let blocked: fn(&ConductDecision) -> GuardrailFailure = recorded_failure;
    let mut recorded = Vec::new();
    let mut record = |item: &ConductDecision| -> Result<(), GuardrailRecordError> {
        recorded.push(item.clone());
        Ok(())
    };
    let mut request_data = BTreeMap::new();
    let mut enforcement = ConductEnforcement {
        request_data: &mut request_data,
        blocked,
        record: &mut record,
    };
    let inputs = text_inputs(&["hello"]);
    let returned = Runtime::new()
        .expect("a runtime")
        .block_on(apply_conduct_guardrail(
            inputs.clone(),
            GuardrailInputType::Request,
            &check,
            &mut enforcement,
        ))
        .expect("an allowing verdict passes");
    assert_eq!(returned, inputs);
    assert_eq!(recorded, vec![decision("allow", Some("rule-2"))]);
}

#[test]
fn conduct_guardrail_apply_skips_the_check_for_a_response_input() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let check = FixedCheck {
        decision: decision("block", None),
        calls: Arc::clone(&calls),
    };
    let blocked: fn(&ConductDecision) -> GuardrailFailure = recorded_failure;
    let mut record = |_item: &ConductDecision| -> Result<(), GuardrailRecordError> { Ok(()) };
    let mut request_data = BTreeMap::new();
    let mut enforcement = ConductEnforcement {
        request_data: &mut request_data,
        blocked,
        record: &mut record,
    };
    let inputs = text_inputs(&["answer"]);
    let returned = Runtime::new()
        .expect("a runtime")
        .block_on(apply_conduct_guardrail(
            inputs.clone(),
            GuardrailInputType::Response,
            &check,
            &mut enforcement,
        ))
        .expect("a response input is never checked");
    assert_eq!(returned, inputs);
    assert!(calls.lock().expect("the call log").is_empty());
}

#[rstest]
#[case::explicit_parameter(&["self", "api_url", "unreachable_fallback"], true)]
#[case::without_the_parameter(&["self", "api_url", "agent_token"], false)]
#[case::kwargs_only(&["self", "kwargs"], false)]
fn conduct_guardrail_binds_unreachable_fallback_inspects_the_constructor(
    #[case] parameters: &[&str],
    #[case] expected: bool,
) {
    let owned: Vec<String> = parameters
        .iter()
        .map(|parameter| (*parameter).to_owned())
        .collect();
    assert_eq!(binds_unreachable_fallback(&owned), expected);
}

#[test]
fn conduct_guardrail_blocking_and_flagged_verdict_sets_are_the_litellm_ones() {
    assert_eq!(BLOCKING_VERDICTS, ["block", "approval"]);
    assert_eq!(FLAGGED_VERDICTS, ["warning", "advisory"]);
}

#[test]
fn conduct_guardrail_missing_package_error_carries_the_litellm_message() {
    assert_eq!(
        ConductCheckError::MissingPackage.to_string(),
        "conduct-litellm-guard>=0.2.5 is required for the Conduct guardrail. Install it with: pip install \"conduct-litellm-guard>=0.2.5\""
    );
}
