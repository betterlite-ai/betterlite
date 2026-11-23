use betterlite_models::decisions::Json;
use tracing::Level;

use super::support::{
    capture_logs, cost, cost_json, details, end, harness, harness_returning, json, kwargs, payload,
    payload_input, payload_json, spent_object, start,
};
use crate::logging_success::payload_state::STANDARD_LOGGING_OBJECT;
use crate::passthrough_handler::types::PASSTHROUGH_LOGGING_PAYLOAD;

#[test]
fn stores_the_built_payload() {
    let harness = harness_returning(payload());
    harness.cost.costs(cost("0.5"));
    let mut response = json(r#"{"id": "chatcmpl-9"}"#);
    let written = harness
        .handler
        .build_response_logging_payload(payload_input(
            &harness.logging,
            &mut response,
            "claude-3",
            kwargs(&[]),
        ));
    assert_eq!(written.entries[STANDARD_LOGGING_OBJECT], payload_json());
    let Json::Object(object) = &written.entries[STANDARD_LOGGING_OBJECT] else {
        panic!("the standard logging object must be an object");
    };
    assert_eq!(object["id"], Json::Text("chatcmpl-1".to_owned()));
    assert_eq!(object["model"], Json::Text("gpt-4o".to_owned()));
    assert_eq!(object["status"], Json::Text("success".to_owned()));
}

#[test]
fn stores_null_when_the_builder_builds_nothing() {
    let harness = harness();
    harness.cost.costs(cost("0.5"));
    let mut response = json(r#"{"id": "chatcmpl-9"}"#);
    let written = harness
        .handler
        .build_response_logging_payload(payload_input(
            &harness.logging,
            &mut response,
            "claude-3",
            kwargs(&[]),
        ));
    assert_eq!(written.entries[STANDARD_LOGGING_OBJECT], Json::Null);
}

#[test]
fn the_builder_sees_the_cost_the_user_and_the_model() {
    let harness = harness_returning(payload());
    harness.cost.costs(cost("0.5"));
    let mut response = json(r#"{"id": "upstream-9"}"#);
    let payload_value = json(r#"{"request_body": {"user": "alice"}}"#);
    let _written = harness
        .handler
        .build_response_logging_payload(payload_input(
            &harness.logging,
            &mut response,
            "claude-3",
            kwargs(&[(PASSTHROUGH_LOGGING_PAYLOAD, payload_value)]),
        ));
    let calls = harness.builder.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "claude-3");
    assert_eq!(calls[0].response, json(r#"{"id": "upstream-9"}"#));
    assert_eq!(calls[0].start_time, start());
    assert_eq!(calls[0].end_time, end());
    // the stamp writes model_call_details["model"] after the build, so the builder sees the init model
    assert_eq!(
        calls[0].details_model,
        Some(Json::Text("gpt-4o".to_owned()))
    );
    assert_eq!(calls[0].cost, Some(cost_json(cost("0.5"))));
    assert_eq!(calls[0].user, Some(Json::Text("alice".to_owned())));
    assert_eq!(
        details(&harness.logging)["model"],
        Json::Text("claude-3".to_owned())
    );
}

#[test]
fn logs_the_object_at_debug_level() {
    let harness = harness_returning(payload());
    harness.cost.costs(cost("0.5"));
    let (captured, _guard) = capture_logs(Level::DEBUG);
    let mut response = json(r#"{"id": "chatcmpl-9"}"#);
    let _written = harness
        .handler
        .build_response_logging_payload(payload_input(
            &harness.logging,
            &mut response,
            "claude-3",
            kwargs(&[]),
        ));
    let text = captured.text();
    assert!(text.contains("standard_logging_object= "));
    assert!(text.contains(r#""model": "gpt-4o""#));
}

#[test]
fn keeps_the_debug_line_off_at_info_level() {
    let harness = harness_returning(payload());
    harness.cost.costs(cost("0.5"));
    let (captured, _guard) = capture_logs(Level::INFO);
    let mut response = json(r#"{"id": "chatcmpl-9"}"#);
    let _written = harness
        .handler
        .build_response_logging_payload(payload_input(
            &harness.logging,
            &mut response,
            "claude-3",
            kwargs(&[]),
        ));
    assert!(!captured.text().contains("standard_logging_object"));
}

#[test]
fn a_spent_state_aborts_the_object() {
    let harness = harness_returning(payload());
    harness.cost.costs(cost("0.5"));
    let (captured, _guard) = capture_logs(Level::ERROR);
    let spent = spent_object();
    let mut response = json(r#"{"id": "chatcmpl-9"}"#);
    let written = harness
        .handler
        .build_response_logging_payload(payload_input(
            &spent,
            &mut response,
            "claude-3",
            kwargs(&[]),
        ));
    assert_eq!(written.entries["response_cost"], cost_json(cost("0.5")));
    assert!(!written.entries.contains_key(STANDARD_LOGGING_OBJECT));
    assert!(harness.builder.calls().is_empty());
    assert_eq!(response, json(r#"{"id": "chatcmpl-9"}"#));
    assert!(
        captured
            .text()
            .contains("the logging state is not available")
    );
}

#[test]
fn a_non_object_response_keeps_the_object_but_not_the_stamp() {
    let harness = harness_returning(payload());
    harness.cost.costs(cost("0.5"));
    let (captured, _guard) = capture_logs(Level::ERROR);
    let mut response = json(r#"["not", "an", "object"]"#);
    let written = harness
        .handler
        .build_response_logging_payload(payload_input(
            &harness.logging,
            &mut response,
            "claude-3",
            kwargs(&[]),
        ));
    assert_eq!(written.entries[STANDARD_LOGGING_OBJECT], payload_json());
    assert_eq!(response, json(r#"["not", "an", "object"]"#));
    let details = details(&harness.logging);
    assert!(details.contains_key("response_cost"));
    assert_eq!(details["model"], Json::Text("gpt-4o".to_owned()));
    assert!(
        captured
            .text()
            .contains("the transformed response is not a JSON object")
    );
}
