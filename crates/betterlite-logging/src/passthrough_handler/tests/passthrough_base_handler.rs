use super::support::{
    chat_input, cost, cost_json, details, end, harness_returning, headers, json, json_object,
    kwargs, payload, payload_json, start, state_model, state_params,
};
use crate::logging_success::payload_state::STANDARD_LOGGING_OBJECT;
use crate::passthrough_handler::types::{
    BODY, CollectedChunksInput, LITELLM_PARAMS, MODEL, NO_COMPLETE_RESPONSE,
    PASSTHROUGH_LOGGING_PAYLOAD, PAYLOAD_ERROR, PROXY_SERVER_REQUEST, PassthroughLoggingPayload,
    RESPONSE_COST, StreamedResponse, USER,
};
use crate::passthrough_logging_state::response::PassThroughResponse;

#[test]
fn writes_only_the_litellm_kwargs() {
    let harness = harness_returning(payload());
    harness
        .provider
        .transform_ok(json(r#"{"id": "chatcmpl-9"}"#));
    harness.cost.costs(cost("0.5"));
    let headers = headers();
    let request = json_object(r#"{"model": "claude-3"}"#);
    let response = json_object(r#"{"type": "message"}"#);
    let passthrough = json(
        r#"{"url": "https://upstream.example/v1/messages", "request_body": {"user": "alice"}}"#,
    );
    let input = chat_input(
        &harness.logging,
        &headers,
        &request,
        &response,
        kwargs(&[(PASSTHROUGH_LOGGING_PAYLOAD, passthrough)]),
    );
    let outcome = harness.handler.passthrough_chat_handler(input).unwrap();
    assert_eq!(
        outcome.kwargs.entries[RESPONSE_COST],
        cost_json(cost("0.5"))
    );
    assert_eq!(
        outcome.kwargs.entries[LITELLM_PARAMS],
        json(r#"{"proxy_server_request": {"body": {"user": "alice"}}}"#)
    );
    assert_eq!(
        outcome.kwargs.entries[STANDARD_LOGGING_OBJECT],
        payload_json()
    );
}

#[test]
fn writes_the_kwargs_in_litellm_order() {
    let harness = harness_returning(payload());
    harness
        .provider
        .transform_ok(json(r#"{"id": "chatcmpl-9"}"#));
    harness.cost.costs(cost("0.5"));
    let headers = headers();
    let request = json_object(r#"{"model": "claude-3"}"#);
    let response = json_object(r#"{"type": "message"}"#);
    let passthrough = json(
        r#"{"url": "https://upstream.example/v1/messages", "request_body": {"user": "alice"}}"#,
    );
    let input = chat_input(
        &harness.logging,
        &headers,
        &request,
        &response,
        kwargs(&[(PASSTHROUGH_LOGGING_PAYLOAD, passthrough)]),
    );
    let outcome = harness.handler.passthrough_chat_handler(input).unwrap();
    let keys: Vec<&str> = outcome.kwargs.entries.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        vec![
            PASSTHROUGH_LOGGING_PAYLOAD,
            RESPONSE_COST,
            MODEL,
            LITELLM_PARAMS,
            STANDARD_LOGGING_OBJECT
        ]
    );
}

#[test]
fn returns_the_transformed_response() {
    let harness = harness_returning(payload());
    harness
        .provider
        .transform_ok(json(r#"{"id": "chatcmpl-9"}"#));
    harness.cost.costs(cost("0.5"));
    let headers = headers();
    let request = json_object(r#"{"model": "claude-3"}"#);
    let response = json_object(r#"{"type": "message"}"#);
    let input = chat_input(&harness.logging, &headers, &request, &response, kwargs(&[]));
    let outcome = harness.handler.passthrough_chat_handler(input).unwrap();
    assert_eq!(
        outcome.result,
        Some(PassThroughResponse::ModelResponse(json(
            r#"{"id": "call-1", "model": "claude-3"}"#
        )))
    );
}

#[test]
fn mirrors_the_writes_into_the_logging_state() {
    let harness = harness_returning(payload());
    harness
        .provider
        .transform_ok(json(r#"{"id": "chatcmpl-9"}"#));
    harness.cost.costs(cost("0.5"));
    let headers = headers();
    let request = json_object(r#"{"model": "claude-3"}"#);
    let response = json_object(r#"{"type": "message"}"#);
    let passthrough = json(r#"{"request_body": {"user": "alice"}}"#);
    let input = chat_input(
        &harness.logging,
        &headers,
        &request,
        &response,
        kwargs(&[(PASSTHROUGH_LOGGING_PAYLOAD, passthrough)]),
    );
    harness.handler.passthrough_chat_handler(input).unwrap();
    assert_eq!(state_model(&harness.logging), "claude-3");
    assert_eq!(
        details(&harness.logging)[RESPONSE_COST],
        cost_json(cost("0.5"))
    );
    assert_eq!(
        state_params(&harness.logging)[PROXY_SERVER_REQUEST],
        json(r#"{"body": {"user": "alice"}}"#)
    );
}

#[test]
fn the_names_match_litellm() {
    assert_eq!(PASSTHROUGH_LOGGING_PAYLOAD, "passthrough_logging_payload");
    assert_eq!(
        PAYLOAD_ERROR,
        "Error creating LLM passthrough response logging payload"
    );
    assert_eq!(
        NO_COMPLETE_RESPONSE,
        "Unable to build complete streaming response for Anthropic passthrough endpoint, not logging..."
    );
    assert_eq!(STANDARD_LOGGING_OBJECT, "standard_logging_object");
    assert_eq!(MODEL, "model");
    assert_eq!(RESPONSE_COST, "response_cost");
    assert_eq!(LITELLM_PARAMS, "litellm_params");
    assert_eq!(PROXY_SERVER_REQUEST, "proxy_server_request");
    assert_eq!(BODY, "body");
    assert_eq!(USER, "user");
}

#[test]
fn reads_every_typed_field_of_the_passthrough_payload() {
    let text = r#"{"url": "https://up/v1", "request_method": "POST", "request_body": {"model": "claude-3"}, "response_body": {"id": "x"}, "cost_per_request": 0.25, "future_field": true}"#;
    let payload: PassthroughLoggingPayload = serde_json::from_str(text).unwrap();
    assert_eq!(payload.url.as_deref(), Some("https://up/v1"));
    assert_eq!(payload.request_method.as_deref(), Some("POST"));
    assert_eq!(
        payload.request_body,
        Some(json_object(r#"{"model": "claude-3"}"#))
    );
    assert_eq!(payload.response_body, Some(json_object(r#"{"id": "x"}"#)));
    assert_eq!(payload.cost_per_request, Some(cost("0.25")));
}

#[test]
fn reads_absent_and_null_fields_as_none() {
    let empty: PassthroughLoggingPayload = serde_json::from_str("{}").unwrap();
    let nulled: PassthroughLoggingPayload = serde_json::from_str(
        r#"{"url": null, "request_method": null, "request_body": null, "response_body": null, "cost_per_request": null}"#,
    )
    .unwrap();
    assert_eq!(empty, nulled);
    assert_eq!(empty.url, None);
    assert_eq!(empty.request_body, None);
    assert_eq!(empty.cost_per_request, None);
}

#[test]
fn a_non_object_request_body_names_the_field() {
    let mut decoder = serde_json::Deserializer::from_str(r#"{"request_body": "nope"}"#);
    let parsed: Result<PassthroughLoggingPayload, _> =
        serde_path_to_error::deserialize(&mut decoder);
    let error = parsed.expect_err("request_body must be an object");
    assert!(error.to_string().contains("request_body"));
}

#[test]
fn the_chunk_path_returns_the_same_kwargs() {
    let harness = harness_returning(payload());
    harness
        .provider
        .stream(Ok(Some(StreamedResponse::Model(json(
            r#"{"id": "rebuilt-9"}"#,
        )))));
    harness.cost.costs(cost("0.5"));
    let request = json_object(r#"{"model": "claude-3"}"#);
    let outcome = harness
        .handler
        .handle_collected_chunks(&CollectedChunksInput {
            logging_obj: &harness.logging,
            request_body: &request,
            all_chunks: &[],
            start_time: start(),
            end_time: end(),
        })
        .unwrap();
    let keys: Vec<&str> = outcome.kwargs.entries.keys().map(String::as_str).collect();
    assert_eq!(keys, vec![RESPONSE_COST, MODEL, STANDARD_LOGGING_OBJECT]);
    assert_eq!(
        outcome.kwargs.entries[STANDARD_LOGGING_OBJECT],
        payload_json()
    );
}
