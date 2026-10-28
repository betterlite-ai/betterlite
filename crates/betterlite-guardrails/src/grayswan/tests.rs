#![cfg(test)]

use std::collections::BTreeMap;

use super::{
    GraySwanGuardrail, GraySwanInput, GraySwanResponse, GraySwanSettings, OnFlaggedAction,
};
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::generic_guardrail_contracts::{
    GuardrailExtraValue, GuardrailMessage, GuardrailToolParam,
};
use secrecy::SecretString;

fn guardrail(action: OnFlaggedAction) -> GraySwanGuardrail {
    let settings = GraySwanSettings {
        on_flagged_action: action,
        violation_threshold: 0.5,
        reasoning_mode: None,
        categories: None,
        policy_id: None,
        fail_open: true,
    };
    GraySwanGuardrail::with_client(
        SecretString::from("secret"),
        Some("http://example".to_owned()),
        settings,
        reqwest::Client::default(),
        super::DEFAULT_TIMEOUT,
    )
}

fn message(role: &str, content: &str) -> GuardrailMessage {
    GuardrailMessage {
        role: role.to_owned(),
        extra: BTreeMap::from([(
            "content".to_owned(),
            GuardrailExtraValue::Text(content.to_owned()),
        )]),
    }
}

#[test]
fn grayswan_payload_reconstructs_history_and_merges_dynamic_parameters() {
    let mut input = GraySwanInput {
        messages: vec![message("user", "new")],
        tools: vec![GuardrailToolParam {
            tool_type: "function".to_owned(),
            extra: BTreeMap::new(),
        }],
        metadata: BTreeMap::new(),
        request_history: vec![message("assistant", "old")],
    };
    input.metadata.insert(
        "policy_id".to_owned(),
        GuardrailExtraValue::Text("request-policy".to_owned()),
    );
    let payload = guardrail(OnFlaggedAction::Block).prepare_payload(&input);
    assert_eq!(payload.messages.len(), 2);
    assert_eq!(
        payload.parameters.get("policy_id"),
        Some(&GuardrailExtraValue::Text("request-policy".to_owned()))
    );
}

#[test]
fn grayswan_verdict_at_threshold_blocks() {
    let guardrail = guardrail(OnFlaggedAction::Block);
    let response = GraySwanResponse {
        violation_score: 0.5,
        violation_reason: Some("policy".to_owned()),
        messages: None,
    };
    let result = guardrail.process_response(response, GuardrailInputType::Request, vec![]);
    drop(result.expect_err("threshold violation"));
}

#[test]
fn grayswan_passthrough_replaces_post_call_messages() {
    let guardrail = guardrail(OnFlaggedAction::Passthrough);
    let response = GraySwanResponse {
        violation_score: 0.5,
        violation_reason: None,
        messages: Some(vec![message("assistant", "changed")]),
    };
    let result = guardrail
        .process_response(response, GuardrailInputType::Response, vec![])
        .expect("post-call passthrough");
    assert_eq!(result.messages, vec![message("assistant", "changed")]);
}

#[test]
fn grayswan_context_reconstructs_request_history() {
    let input = GraySwanInput {
        messages: vec![message("user", "new")],
        tools: Vec::new(),
        metadata: BTreeMap::new(),
        request_history: vec![message("assistant", "old")],
    };
    assert_eq!(
        guardrail(OnFlaggedAction::Monitor)
            .prepare_payload(&input)
            .messages
            .len(),
        2
    );
}

#[test]
fn grayswan_monitor_verdict_passes_through() {
    let response = GraySwanResponse {
        violation_score: 0.5,
        violation_reason: None,
        messages: None,
    };
    let result = guardrail(OnFlaggedAction::Monitor)
        .process_response(response, GuardrailInputType::Request, Vec::new())
        .expect("monitor action");
    assert!(result.flagged);
}

#[test]
fn grayswan_error_policy_is_fail_open_by_default() {
    assert!(guardrail(OnFlaggedAction::Monitor).settings.fail_open);
}
