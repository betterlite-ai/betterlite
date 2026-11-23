use betterlite_models::decisions::Json;
use tracing::Level;

use super::support::{capture_logs, cost, harness, json, kwargs, payload_input, state_params};
use crate::logging_success::payload_state::STANDARD_LOGGING_OBJECT;
use crate::passthrough_handler::types::PASSTHROUGH_LOGGING_PAYLOAD;

#[test]
fn writes_the_user_into_litellm_params() {
    let harness = harness();
    harness.cost.costs(cost("0.25"));
    let mut response = json(r#"{"id": "chatcmpl-9"}"#);
    let payload = json(
        r#"{"url": "https://upstream.example/v1/messages", "request_body": {"user": "alice"}}"#,
    );
    let written = harness
        .handler
        .build_response_logging_payload(payload_input(
            &harness.logging,
            &mut response,
            "claude-3",
            kwargs(&[(PASSTHROUGH_LOGGING_PAYLOAD, payload)]),
        ));
    let request = json(r#"{"proxy_server_request": {"body": {"user": "alice"}}}"#);
    assert_eq!(written.entries["litellm_params"], request);
    assert_eq!(
        state_params(&harness.logging)["proxy_server_request"],
        json(r#"{"body": {"user": "alice"}}"#)
    );
}

#[test]
fn keeps_the_other_litellm_params_keys() {
    let harness = harness();
    harness.cost.costs(cost("0.25"));
    let mut response = json(r#"{"id": "chatcmpl-9"}"#);
    let payload = json(r#"{"request_body": {"user": "alice"}}"#);
    let params = json(r#"{"api_base": "https://api", "proxy_server_request": {"url": "/old"}}"#);
    let written = harness
        .handler
        .build_response_logging_payload(payload_input(
            &harness.logging,
            &mut response,
            "claude-3",
            kwargs(&[
                (PASSTHROUGH_LOGGING_PAYLOAD, payload),
                ("litellm_params", params),
            ]),
        ));
    assert_eq!(
        written.entries["litellm_params"],
        json(r#"{"api_base": "https://api", "proxy_server_request": {"body": {"user": "alice"}}}"#)
    );
}

#[test]
fn skips_an_absent_payload() {
    let harness = harness();
    harness.cost.costs(cost("0.25"));
    let mut response = json(r#"{"id": "chatcmpl-9"}"#);
    let written = harness
        .handler
        .build_response_logging_payload(payload_input(
            &harness.logging,
            &mut response,
            "claude-3",
            kwargs(&[]),
        ));
    assert!(!written.entries.contains_key("litellm_params"));
    assert!(state_params(&harness.logging).is_empty());
}

#[test]
fn skips_a_null_payload() {
    let harness = harness();
    harness.cost.costs(cost("0.25"));
    let mut response = json(r#"{"id": "chatcmpl-9"}"#);
    let written = harness
        .handler
        .build_response_logging_payload(payload_input(
            &harness.logging,
            &mut response,
            "claude-3",
            kwargs(&[(PASSTHROUGH_LOGGING_PAYLOAD, Json::Null)]),
        ));
    assert!(!written.entries.contains_key("litellm_params"));
}

#[test]
fn skips_a_payload_without_a_request_body() {
    let harness = harness();
    harness.cost.costs(cost("0.25"));
    let mut response = json(r#"{"id": "chatcmpl-9"}"#);
    let payload = json(r#"{"url": "https://upstream.example/v1/messages"}"#);
    let written = harness
        .handler
        .build_response_logging_payload(payload_input(
            &harness.logging,
            &mut response,
            "claude-3",
            kwargs(&[(PASSTHROUGH_LOGGING_PAYLOAD, payload)]),
        ));
    assert!(!written.entries.contains_key("litellm_params"));
}

#[test]
fn skips_an_empty_user() {
    let harness = harness();
    harness.cost.costs(cost("0.25"));
    let mut response = json(r#"{"id": "chatcmpl-9"}"#);
    let payload = json(r#"{"request_body": {"user": ""}}"#);
    let written = harness
        .handler
        .build_response_logging_payload(payload_input(
            &harness.logging,
            &mut response,
            "claude-3",
            kwargs(&[(PASSTHROUGH_LOGGING_PAYLOAD, payload)]),
        ));
    assert!(!written.entries.contains_key("litellm_params"));
}

#[test]
fn treats_null_litellm_params_as_absent() {
    let harness = harness();
    harness.cost.costs(cost("0.25"));
    let mut response = json(r#"{"id": "chatcmpl-9"}"#);
    let payload = json(r#"{"request_body": {"user": "alice"}}"#);
    let written = harness
        .handler
        .build_response_logging_payload(payload_input(
            &harness.logging,
            &mut response,
            "claude-3",
            kwargs(&[
                (PASSTHROUGH_LOGGING_PAYLOAD, payload),
                ("litellm_params", Json::Null),
            ]),
        ));
    assert_eq!(
        written.entries["litellm_params"],
        json(r#"{"proxy_server_request": {"body": {"user": "alice"}}}"#)
    );
}

#[test]
fn a_malformed_payload_aborts_the_standard_object() {
    let harness = harness();
    harness.cost.costs(cost("0.25"));
    let (captured, _guard) = capture_logs(Level::ERROR);
    let mut response = json(r#"{"id": "chatcmpl-9"}"#);
    let written = harness
        .handler
        .build_response_logging_payload(payload_input(
            &harness.logging,
            &mut response,
            "claude-3",
            kwargs(&[(PASSTHROUGH_LOGGING_PAYLOAD, Json::Text("nope".to_owned()))]),
        ));
    assert_eq!(written.entries["response_cost"], json("0.25"));
    assert!(!written.entries.contains_key("litellm_params"));
    assert!(!written.entries.contains_key(STANDARD_LOGGING_OBJECT));
    assert!(
        captured
            .text()
            .contains("the passthrough logging payload could not be read")
    );
}

#[test]
fn a_non_object_request_body_names_the_field() {
    let harness = harness();
    harness.cost.costs(cost("0.25"));
    let (captured, _guard) = capture_logs(Level::ERROR);
    let mut response = json(r#"{"id": "chatcmpl-9"}"#);
    harness
        .handler
        .build_response_logging_payload(payload_input(
            &harness.logging,
            &mut response,
            "claude-3",
            kwargs(&[(
                PASSTHROUGH_LOGGING_PAYLOAD,
                json(r#"{"request_body": "nope"}"#),
            )]),
        ));
    assert!(captured.text().contains("request_body"));
}

#[test]
fn a_non_object_litellm_params_aborts() {
    let harness = harness();
    harness.cost.costs(cost("0.25"));
    let (captured, _guard) = capture_logs(Level::ERROR);
    let mut response = json(r#"{"id": "chatcmpl-9"}"#);
    let payload = json(r#"{"request_body": {"user": "alice"}}"#);
    let written = harness
        .handler
        .build_response_logging_payload(payload_input(
            &harness.logging,
            &mut response,
            "claude-3",
            kwargs(&[
                (PASSTHROUGH_LOGGING_PAYLOAD, payload),
                ("litellm_params", Json::Text("x".to_owned())),
            ]),
        ));
    assert_eq!(
        written.entries["litellm_params"],
        Json::Text("x".to_owned())
    );
    assert!(!written.entries.contains_key(STANDARD_LOGGING_OBJECT));
    assert!(
        captured
            .text()
            .contains("the litellm_params entry is not a JSON object")
    );
}
