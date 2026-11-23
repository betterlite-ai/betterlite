use betterlite_models::decisions::Json;
use rust_decimal::Decimal;

use super::support::{cost, cost_json, details, harness, json, kwargs, payload_input, state_model};

#[test]
fn writes_the_cost_to_kwargs_and_the_logging_state() {
    let harness = harness();
    harness.cost.costs(cost("0.25"));
    let mut response = json(r#"{"id": "chatcmpl-9"}"#);
    let written = harness
        .handler
        .build_response_logging_payload(payload_input(
            &harness.logging,
            &mut response,
            "claude-3-opus",
            kwargs(&[]),
        ));
    assert_eq!(written.entries["response_cost"], cost_json(cost("0.25")));
    assert_eq!(
        written.entries["model"],
        Json::Text("claude-3-opus".to_owned())
    );
    let details = details(&harness.logging);
    assert_eq!(details["response_cost"], cost_json(cost("0.25")));
    assert_eq!(state_model(&harness.logging), "claude-3-opus");
    let calls = harness.cost.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "claude-3-opus");
    assert_eq!(calls[0].response, json(r#"{"id": "chatcmpl-9"}"#));
}

#[test]
fn writes_a_zero_cost() {
    let harness = harness();
    harness.cost.costs(Decimal::ZERO);
    let mut response = json(r#"{"id": "chatcmpl-9"}"#);
    let written = harness
        .handler
        .build_response_logging_payload(payload_input(
            &harness.logging,
            &mut response,
            "claude-3",
            kwargs(&[]),
        ));
    assert_eq!(written.entries["response_cost"], cost_json(Decimal::ZERO));
    let details = details(&harness.logging);
    assert_eq!(details["response_cost"], cost_json(Decimal::ZERO));
}

#[test]
fn keeps_a_high_precision_cost() {
    let harness = harness();
    harness.cost.costs(cost("0.000123"));
    let mut response = json(r#"{"id": "chatcmpl-9"}"#);
    let written = harness
        .handler
        .build_response_logging_payload(payload_input(
            &harness.logging,
            &mut response,
            "claude-3",
            kwargs(&[]),
        ));
    let text = serde_json::to_string(&written.entries["response_cost"]).unwrap();
    assert_eq!(text, "0.000123");
}

#[test]
fn writes_the_upstream_reported_cost_as_given() {
    let harness = harness();
    harness.cost.costs(cost("0.013"));
    let mut response = json(r#"{"id": "chatcmpl-9", "_hidden_params": {"response_cost": 0.013}}"#);
    let written = harness
        .handler
        .build_response_logging_payload(payload_input(
            &harness.logging,
            &mut response,
            "claude-3",
            kwargs(&[]),
        ));
    let text = serde_json::to_string(&written.entries["response_cost"]).unwrap();
    assert_eq!(text, "0.013");
    assert_eq!(
        harness.cost.calls()[0].response,
        json(r#"{"id": "chatcmpl-9", "_hidden_params": {"response_cost": 0.013}}"#)
    );
}

#[test]
fn a_cost_failure_leaves_the_cost_and_the_rest_unwritten() {
    let harness = harness();
    harness.cost.fails();
    let mut response = json(r#"{"id": "chatcmpl-9"}"#);
    let written = harness
        .handler
        .build_response_logging_payload(payload_input(
            &harness.logging,
            &mut response,
            "claude-3",
            kwargs(&[]),
        ));
    assert!(written.entries.is_empty());
    assert!(!details(&harness.logging).contains_key("response_cost"));
    assert_eq!(state_model(&harness.logging), "gpt-4o");
    assert_eq!(response, json(r#"{"id": "chatcmpl-9"}"#));
}

#[test]
fn stamps_the_call_id_and_the_model() {
    let harness = harness();
    harness.cost.costs(cost("0.25"));
    let mut response = json(r#"{"id": "upstream-9", "object": "chat.completion"}"#);
    let _written = harness
        .handler
        .build_response_logging_payload(payload_input(
            &harness.logging,
            &mut response,
            "claude-3",
            kwargs(&[]),
        ));
    assert_eq!(
        response,
        json(r#"{"id": "call-1", "object": "chat.completion", "model": "claude-3"}"#)
    );
    let details = details(&harness.logging);
    assert_eq!(details["model"], Json::Text("claude-3".to_owned()));
}
