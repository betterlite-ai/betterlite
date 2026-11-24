use betterlite_models::decisions::Json;
use tracing::Level;

use super::support::{
    capture_logs, chat_input, cost, harness, headers, json, json_object, kwargs, payload_input,
};
use crate::logging_success::payload_state::STANDARD_LOGGING_OBJECT;
use crate::passthrough_handler::types::{PASSTHROUGH_LOGGING_PAYLOAD, PAYLOAD_ERROR};
use crate::passthrough_logging_state::response::PassThroughResponse;

#[test]
fn returns_the_kwargs_it_has_when_the_cost_fails() {
    let harness = harness();
    harness.cost.fails();
    let (captured, _guard) = capture_logs(Level::ERROR);
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
    assert!(
        captured
            .text()
            .contains(&format!("{PAYLOAD_ERROR}: the fake probe failed"))
    );
}

#[test]
fn the_chat_handler_still_returns_the_response() {
    let harness = harness();
    harness
        .provider
        .transform_ok(json(r#"{"id": "chatcmpl-9"}"#));
    harness.cost.fails();
    let (captured, _guard) = capture_logs(Level::ERROR);
    let headers = headers();
    let request = json_object(r#"{"model": "claude-3"}"#);
    let response = json_object(r#"{"type": "message"}"#);
    let input = chat_input(&harness.logging, &headers, &request, &response, kwargs(&[]));
    let outcome = harness.handler.passthrough_chat_handler(input).unwrap();
    assert_eq!(
        outcome.result,
        Some(PassThroughResponse::ModelResponse(json(
            r#"{"id": "chatcmpl-9"}"#
        )))
    );
    assert!(outcome.kwargs.entries.is_empty());
    assert!(captured.text().contains(PAYLOAD_ERROR));
}

#[test]
fn a_malformed_payload_stays_inside_the_build() {
    let harness = harness();
    harness
        .provider
        .transform_ok(json(r#"{"id": "chatcmpl-9"}"#));
    harness.cost.costs(cost("0.25"));
    let (captured, _guard) = capture_logs(Level::ERROR);
    let headers = headers();
    let request = json_object(r#"{"model": "claude-3"}"#);
    let response = json_object(r#"{"type": "message"}"#);
    let payload = Json::Text("not-a-payload".to_owned());
    let input = chat_input(
        &harness.logging,
        &headers,
        &request,
        &response,
        kwargs(&[(PASSTHROUGH_LOGGING_PAYLOAD, payload)]),
    );
    let outcome = harness.handler.passthrough_chat_handler(input).unwrap();
    assert_eq!(
        outcome.result,
        Some(PassThroughResponse::ModelResponse(json(
            r#"{"id": "chatcmpl-9"}"#
        )))
    );
    assert!(outcome.kwargs.entries.contains_key("response_cost"));
    assert!(!outcome.kwargs.entries.contains_key(STANDARD_LOGGING_OBJECT));
    assert!(captured.text().contains("could not be read"));
}

#[test]
fn a_transform_error_is_not_caught() {
    let harness = harness();
    harness.provider.transform_fails();
    let (captured, _guard) = capture_logs(Level::ERROR);
    let headers = headers();
    let request = json_object(r#"{"model": "claude-3"}"#);
    let response = json_object(r#"{"type": "message"}"#);
    let input = chat_input(&harness.logging, &headers, &request, &response, kwargs(&[]));
    let error = harness
        .handler
        .passthrough_chat_handler(input)
        .expect_err("only the payload build is caught");
    assert_eq!(error.to_string(), "the fake probe failed");
    assert!(captured.text().is_empty());
}

#[test]
fn names_the_field_of_a_wrong_typed_model() {
    let harness = harness();
    let headers = headers();
    let request = json_object(r#"{"model": 7}"#);
    let response = json_object(r#"{"model": "upstream"}"#);
    let input = chat_input(&harness.logging, &headers, &request, &response, kwargs(&[]));
    let error = harness
        .handler
        .passthrough_chat_handler(input)
        .expect_err("a number is not a model");
    assert_eq!(
        error.to_string(),
        "the request_body.model value must be a string"
    );
}
