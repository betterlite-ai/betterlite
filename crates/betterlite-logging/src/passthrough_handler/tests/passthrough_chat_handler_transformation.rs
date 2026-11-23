use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::{chat_input, cost, cost_json, harness, headers, json, json_object, kwargs};
use crate::passthrough_handler::types::UpstreamChatResponse;
use crate::passthrough_logging_state::response::PassThroughResponse;

#[test]
fn transforms_through_the_provider_config() {
    let harness = harness();
    harness
        .provider
        .transform_ok(json(r#"{"id": "chatcmpl-9", "object": "chat.completion"}"#));
    harness.cost.costs(cost("0.25"));
    let headers = headers();
    let request = json_object(r#"{"model": "claude-3", "max_tokens": 16}"#);
    let response = json_object(r#"{"type": "message"}"#);
    let input = chat_input(&harness.logging, &headers, &request, &response, kwargs(&[]));
    let outcome = harness.handler.passthrough_chat_handler(input).unwrap();
    let calls = harness.provider.transforms();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "claude-3");
    assert_eq!(calls[0].status, 200);
    assert_eq!(calls[0].content_type.as_deref(), Some("application/json"));
    assert_eq!(calls[0].body, b"{}".to_vec());
    assert_eq!(calls[0].call_id.as_deref(), Some("call-1"));
    assert_eq!(
        outcome.result,
        Some(PassThroughResponse::ModelResponse(json(
            r#"{"id": "call-1", "object": "chat.completion", "model": "claude-3"}"#
        )))
    );
    assert_eq!(
        outcome.kwargs.entries.get("response_cost"),
        Some(&cost_json(cost("0.25")))
    );
    assert_eq!(
        outcome.kwargs.entries.get("model"),
        Some(&Json::Text("claude-3".to_owned()))
    );
    assert_eq!(
        outcome.kwargs.entries.get("standard_logging_object"),
        Some(&Json::Null)
    );
}

#[rstest]
#[case::request_wins(r#"{"model": "claude-3"}"#, r#"{"model": "upstream"}"#, "claude-3")]
#[case::response_fallback(r#"{"max_tokens": 16}"#, r#"{"model": "upstream"}"#, "upstream")]
#[case::null_request_model(r#"{"model": null}"#, r#"{"model": "upstream"}"#, "upstream")]
#[case::null_response_model(r#"{"max_tokens": 16}"#, r#"{"model": null}"#, "")]
#[case::neither_has_a_model(r#"{"max_tokens": 16}"#, r#"{"type": "message"}"#, "")]
fn picks_the_model_from_the_bodies(
    #[case] request: &str,
    #[case] response: &str,
    #[case] expected: &str,
) {
    let harness = harness();
    harness
        .provider
        .transform_ok(json(r#"{"id": "chatcmpl-1"}"#));
    harness.cost.costs(cost("0.25"));
    let headers = headers();
    let request_body = json_object(request);
    let response_body = json_object(response);
    let input = chat_input(
        &harness.logging,
        &headers,
        &request_body,
        &response_body,
        kwargs(&[]),
    );
    let outcome = harness.handler.passthrough_chat_handler(input).unwrap();
    assert_eq!(harness.provider.config_models(), vec![expected.to_owned()]);
    assert_eq!(harness.provider.transforms()[0].model, expected);
    assert_eq!(harness.cost.calls()[0].model, expected);
    assert_eq!(
        outcome.kwargs.entries["model"],
        Json::Text(expected.to_owned())
    );
}

#[rstest]
#[case::request(r#"{"model": 7}"#, r#"{"model": "upstream"}"#, "request_body.model")]
#[case::response(r#"{"max_tokens": 16}"#, r#"{"model": true}"#, "response_body.model")]
fn refuses_a_model_of_the_wrong_type(
    #[case] request: &str,
    #[case] response: &str,
    #[case] field: &str,
) {
    let harness = harness();
    let headers = headers();
    let request_body = json_object(request);
    let response_body = json_object(response);
    let input = chat_input(
        &harness.logging,
        &headers,
        &request_body,
        &response_body,
        kwargs(&[]),
    );
    let error = harness
        .handler
        .passthrough_chat_handler(input)
        .expect_err("a non-text model is refused");
    assert_eq!(
        error.to_string(),
        format!("the {field} value must be a string")
    );
    assert!(harness.provider.config_models().is_empty());
    assert!(harness.provider.transforms().is_empty());
}

#[test]
fn records_the_upstream_status_and_body() {
    let harness = harness();
    harness
        .provider
        .transform_ok(json(r#"{"id": "chatcmpl-1"}"#));
    harness.cost.costs(cost("0.25"));
    let headers = headers();
    let request = json_object(r#"{"model": "claude-3"}"#);
    let response = json_object(r#"{"type": "error"}"#);
    let mut input = chat_input(&harness.logging, &headers, &request, &response, kwargs(&[]));
    input.upstream = UpstreamChatResponse {
        status_code: 529,
        headers: &headers,
        body: b"overloaded",
    };
    harness.handler.passthrough_chat_handler(input).unwrap();
    let calls = harness.provider.transforms();
    assert_eq!(calls[0].status, 529);
    assert_eq!(calls[0].body, b"overloaded".to_vec());
}

#[test]
fn propagates_a_transform_error() {
    let harness = harness();
    harness.provider.transform_fails();
    let headers = headers();
    let request = json_object(r#"{"model": "claude-3"}"#);
    let response = json_object(r#"{"type": "message"}"#);
    let input = chat_input(&harness.logging, &headers, &request, &response, kwargs(&[]));
    let error = harness
        .handler
        .passthrough_chat_handler(input)
        .expect_err("the transform failure is not caught");
    assert_eq!(error.to_string(), "the fake probe failed");
    assert!(harness.cost.calls().is_empty());
}

#[test]
fn keeps_the_caller_kwargs() {
    let harness = harness();
    harness
        .provider
        .transform_ok(json(r#"{"id": "chatcmpl-1"}"#));
    harness.cost.costs(cost("0.25"));
    let headers = headers();
    let request = json_object(r#"{"model": "claude-3"}"#);
    let response = json_object(r#"{"type": "message"}"#);
    let input = chat_input(
        &harness.logging,
        &headers,
        &request,
        &response,
        kwargs(&[("custom_llm_provider", Json::Text("anthropic".to_owned()))]),
    );
    let outcome = harness.handler.passthrough_chat_handler(input).unwrap();
    assert_eq!(
        outcome.kwargs.entries["custom_llm_provider"],
        Json::Text("anthropic".to_owned())
    );
}
