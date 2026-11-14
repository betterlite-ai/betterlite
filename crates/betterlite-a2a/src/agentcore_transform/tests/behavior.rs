use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;
use futures_core::Stream;
use futures_util::{StreamExt as _, stream};
use rstest::rstest;
use secrecy::ExposeSecret as _;
use serde_json::json;

use crate::agentcore_transform::call::{AgentCoreHandler, HandlerCall, SignedRequestCall};
use crate::agentcore_transform::config::{BedrockAgentCoreConfig, ConfigCall};
use crate::agentcore_transform::headers::filter_reserved_headers;
use crate::agentcore_transform::params::LitellmParams;
use crate::agentcore_transform::runtime::{MAX_RESPONSE_BYTES, RpcMethod, collect_body};
use crate::agentcore_transform::tests::support::{
    CONTEXT_ID, FixtureError, FixtureRuntime, GENERATED, SESSION_PREFIX, Source, URL,
    bearer_params, fixture, litellm_params, params, parsed,
};
use crate::agentcore_transform::{
    AgentCoreError, SignedRequest, TransformError, get_url_and_signed_request, parse_sse_events,
    scoped_session_id, validate_runtime_session_id,
};
use crate::completion_bridge::wire::MessageSendParams;

const SESSION_HEADER: &str = "X-Amzn-Bedrock-AgentCore-Runtime-Session-Id";
const USER_HEADER: &str = "X-Amzn-Bedrock-AgentCore-Runtime-User-Id";
const SCOPED_PREFIX: &str = "7c36b0a9dedde119";
const MISSING_PARAMS: &str = "litellm_params is required for BedrockAgentCoreA2AConfig (must contain model with AgentCore ARN)";

fn handler_call<'call>(
    request: &'call MessageSendParams,
    litellm_params: &'call LitellmParams,
) -> HandlerCall<'call> {
    HandlerCall {
        request_id: "req-1",
        params: request,
        litellm_params: litellm_params.clone(),
        agent_extra_headers: None,
    }
}

const fn config_call<'call>(
    request: &'call MessageSendParams,
    litellm_params: Option<&'call BTreeMap<String, AgentJson>>,
) -> ConfigCall<'call> {
    ConfigCall {
        request_id: "req-1",
        params: request,
        api_base: None,
        litellm_params,
        agent_extra_headers: None,
    }
}

async fn signed_request(
    runtime: &FixtureRuntime,
    request: &MessageSendParams,
    litellm_params: &LitellmParams,
    method: RpcMethod,
) -> Result<SignedRequest, AgentCoreError<FixtureError>> {
    get_url_and_signed_request(
        runtime,
        &SignedRequestCall {
            request_id: "req-1",
            params: request,
            litellm_params,
            method,
            agent_extra_headers: None,
        },
        &mut Source,
    )
    .await
}

fn envelope_body(result: &Result<SignedRequest, AgentCoreError<FixtureError>>) -> AgentJson {
    let request = result.as_ref().expect("the request is signed");
    serde_json::from_slice(&request.body).expect("the signed body is JSON")
}

async fn collected<S>(stream: S) -> Vec<Result<AgentJson, AgentCoreError<FixtureError>>>
where
    S: Stream<Item = Result<AgentJson, AgentCoreError<FixtureError>>> + Send,
{
    let mut pinned = std::pin::pin!(stream);
    let mut items = Vec::new();
    while let Some(item) = pinned.next().await {
        items.push(item);
    }
    items
}

fn values(items: Vec<Result<AgentJson, AgentCoreError<FixtureError>>>) -> Vec<AgentJson> {
    items
        .into_iter()
        .map(|item| item.expect("the event is not an error"))
        .collect()
}

fn valid_lines(items: &[&str]) -> impl Stream<Item = Result<String, FixtureError>> {
    stream::iter(items.iter().map(|line| Ok((*line).to_owned())))
}

#[rstest]
#[case::authorization("authorization")]
#[case::host("host")]
#[case::amz_prefix("x-amz-date")]
#[case::runtime_prefix("x-amzn-bedrock-agentcore-runtime-user-id")]
#[case::capitalized_authorization("Authorization")]
#[case::capitalized_prefix("X-Amz-Security-Token")]
fn reserved_headers_are_dropped_in_any_case(#[case] name: &str) {
    let headers = BTreeMap::from([(name.to_owned(), "value".to_owned())]);

    let filtered = filter_reserved_headers(&headers);

    assert!(filtered.is_empty());
}

#[test]
fn safe_headers_survive_the_filter() {
    let headers = BTreeMap::from([
        ("x-request-id".to_owned(), "abc".to_owned()),
        ("Authorization".to_owned(), "Bearer fake".to_owned()),
    ]);

    let filtered = filter_reserved_headers(&headers);

    assert_eq!(
        filtered,
        BTreeMap::from([("x-request-id".to_owned(), "abc".to_owned())])
    );
}

#[test]
fn an_empty_header_map_filters_to_nothing() {
    assert!(filter_reserved_headers(&BTreeMap::new()).is_empty());
}

#[test]
fn the_context_id_becomes_a_principal_scoped_session() {
    let request = params(Some(json!(CONTEXT_ID)));

    let derived = scoped_session_id(&request, Some("key-2"));

    assert_eq!(derived, Some(format!("{SCOPED_PREFIX}-{CONTEXT_ID}")));
}

#[test]
fn a_context_id_without_a_principal_keeps_its_value() {
    let request = params(Some(json!(CONTEXT_ID)));

    let derived = scoped_session_id(&request, None);

    assert_eq!(derived, Some(CONTEXT_ID.to_owned()));
}

#[test]
fn a_missing_context_id_has_no_session() {
    let request = params(None);

    let derived = scoped_session_id(&request, None);

    assert_eq!(derived, None);
}

#[test]
fn a_non_text_context_id_has_no_session() {
    let request = params(Some(parsed("5")));

    let derived = scoped_session_id(&request, None);

    assert_eq!(derived, None);
}

#[test]
fn an_empty_context_id_has_no_session() {
    let request = params(Some(json!("")));

    let derived = scoped_session_id(&request, None);

    assert_eq!(derived, None);
}

#[rstest]
#[case::minimum(33, true)]
#[case::maximum(256, true)]
#[case::below_minimum(32, false)]
#[case::above_maximum(257, false)]
fn the_session_bounds_are_33_to_256(#[case] length: usize, #[case] accepted: bool) {
    let session_id = "s".repeat(length);

    let validated = validate_runtime_session_id(&session_id);

    assert_eq!(validated.is_ok(), accepted);
}

#[tokio::test]
async fn the_envelope_names_jsonrpc_2_0_method_id_and_params() {
    let runtime = fixture();
    let request = params(Some(json!(CONTEXT_ID)));
    let litellm = bearer_params();

    let result = signed_request(&runtime, &request, &litellm, RpcMethod::Send).await;
    let body = envelope_body(&result);

    assert_eq!(body["jsonrpc"], json!("2.0"));
    assert_eq!(body["method"], json!("message/send"));
    assert_eq!(body["id"], json!("req-1"));
    assert_eq!(body["params"]["message"]["messageId"], json!("msg-1"));
    assert_eq!(body["params"]["message"]["contextId"], json!(CONTEXT_ID));
}

#[tokio::test]
async fn the_streaming_envelope_uses_message_stream() {
    let runtime = fixture();
    let request = params(Some(json!(CONTEXT_ID)));
    let litellm = bearer_params();

    let result = signed_request(&runtime, &request, &litellm, RpcMethod::Stream).await;
    let body = envelope_body(&result);

    assert_eq!(body["method"], json!("message/stream"));
}

#[tokio::test]
async fn the_bedrock_prefix_is_stripped_before_the_runtime() {
    let runtime = fixture();
    let request = params(None);
    let litellm = litellm_params(&[
        ("model", json!("bedrock/agentcore/arn")),
        ("api_key", json!("jwt-token")),
    ]);

    let result = signed_request(&runtime, &request, &litellm, RpcMethod::Send).await;

    assert_eq!(
        result.expect("the request is signed").url,
        format!("{URL}agentcore/arn")
    );
    let signing = runtime.signing.lock().expect("the fixture lock");
    assert_eq!(signing[0].model, "agentcore/arn");
}

#[tokio::test]
async fn a_model_without_the_prefix_reaches_the_runtime_unchanged() {
    let runtime = fixture();
    let request = params(None);
    let litellm = litellm_params(&[
        ("model", json!("agentcore/arn")),
        ("api_key", json!("jwt-token")),
    ]);

    let result = signed_request(&runtime, &request, &litellm, RpcMethod::Send).await;

    assert_eq!(
        result.expect("the request is signed").url,
        format!("{URL}agentcore/arn")
    );
}

#[tokio::test]
async fn the_session_and_user_headers_are_set_before_signing() {
    let mut runtime = fixture();
    runtime.user = Some("user-1".to_owned());
    let request = params(Some(json!(CONTEXT_ID)));
    let litellm = bearer_params();
    let expected_session = format!("{SCOPED_PREFIX}-{CONTEXT_ID}");

    let result = signed_request(&runtime, &request, &litellm, RpcMethod::Send).await;

    let signed = result.expect("the request is signed");
    assert_eq!(
        signed.headers.get(SESSION_HEADER).map(String::as_str),
        Some(expected_session.as_str())
    );
    assert_eq!(
        signed.headers.get(USER_HEADER).map(String::as_str),
        Some("user-1")
    );
    let signing = runtime.signing.lock().expect("the fixture lock");
    assert_eq!(signing[0].headers, signed.headers);
}

#[tokio::test]
async fn an_empty_user_id_sets_no_user_header() {
    let mut runtime = fixture();
    runtime.user = Some(String::new());
    let request = params(Some(json!(CONTEXT_ID)));
    let litellm = bearer_params();

    let result = signed_request(&runtime, &request, &litellm, RpcMethod::Send).await;

    assert!(
        !result
            .expect("the request is signed")
            .headers
            .contains_key(USER_HEADER)
    );
}

#[tokio::test]
async fn the_signature_sees_the_api_key_and_the_model() {
    let runtime = fixture();
    let request = params(Some(json!(CONTEXT_ID)));
    let litellm = bearer_params();

    let result = signed_request(&runtime, &request, &litellm, RpcMethod::Send).await;

    result.as_ref().expect("the request is signed");
    let signing = runtime.signing.lock().expect("the fixture lock");
    let api_key = signing[0].api_key.as_ref().expect("the api key is signed");
    assert_eq!(api_key.expose_secret(), "jwt-token");
    assert_eq!(signing[0].model, "agentcore/arn");
}

#[tokio::test]
async fn the_generated_session_id_is_used_when_no_context_id_is_given() {
    let runtime = fixture();
    let request = params(None);
    let litellm = bearer_params();
    let expected_session = format!("{SESSION_PREFIX}-{GENERATED}");

    let result = signed_request(&runtime, &request, &litellm, RpcMethod::Send).await;

    assert_eq!(
        result
            .expect("the request is signed")
            .headers
            .get(SESSION_HEADER)
            .map(String::as_str),
        Some(expected_session.as_str())
    );
}

#[tokio::test]
async fn a_short_scoped_session_id_is_a_bad_request() {
    let runtime = fixture();
    let request = params(Some(json!("short")));
    let litellm = bearer_params();

    let result = signed_request(&runtime, &request, &litellm, RpcMethod::Send).await;

    assert!(matches!(
        result,
        Err(AgentCoreError::Transform(TransformError::SessionId { .. }))
    ));
}

#[tokio::test]
async fn a_runtime_error_is_carried() {
    let mut runtime = fixture();
    runtime.url_error = Some("the model is not an AgentCore ARN");
    let request = params(Some(json!(CONTEXT_ID)));
    let litellm = bearer_params();

    let result = signed_request(&runtime, &request, &litellm, RpcMethod::Send).await;

    assert!(matches!(
        result,
        Err(AgentCoreError::Runtime(FixtureError(_)))
    ));
}

#[tokio::test]
async fn extra_headers_are_signed_without_the_reserved_ones() {
    let runtime = fixture();
    let request = params(Some(json!(CONTEXT_ID)));
    let litellm = bearer_params();
    let extra = BTreeMap::from([
        ("x-request-id".to_owned(), "abc".to_owned()),
        ("Authorization".to_owned(), "Bearer fake".to_owned()),
        ("x-amz-date".to_owned(), "20260101T000000Z".to_owned()),
    ]);

    let result = get_url_and_signed_request(
        &runtime,
        &SignedRequestCall {
            request_id: "req-1",
            params: &request,
            litellm_params: &litellm,
            method: RpcMethod::Send,
            agent_extra_headers: Some(&extra),
        },
        &mut Source,
    )
    .await;

    let headers = result.expect("the request is signed").headers;
    assert_eq!(headers.get("x-request-id").map(String::as_str), Some("abc"));
    assert!(!headers.contains_key("Authorization"));
    assert!(!headers.contains_key("x-amz-date"));
}

#[tokio::test]
async fn valid_data_lines_become_json_events() {
    let events = collected(parse_sse_events(valid_lines(&[
        "data: {\"a\": 1}",
        "",
        "data: {\"b\": 2}",
    ])))
    .await;

    assert_eq!(
        values(events),
        vec![parsed(r#"{"a":1}"#), parsed(r#"{"b":2}"#)]
    );
}

#[tokio::test]
async fn a_non_json_data_line_is_skipped() {
    let events = collected(parse_sse_events(valid_lines(&[
        "data: not-json",
        "data: {\"ok\": true}",
    ])))
    .await;

    assert_eq!(values(events), vec![parsed(r#"{"ok":true}"#)]);
}

#[tokio::test]
async fn an_empty_data_payload_is_skipped() {
    let events = collected(parse_sse_events(valid_lines(&[
        "data:", "data:   ", "data: 5",
    ])))
    .await;

    assert_eq!(values(events), vec![parsed("5")]);
}

#[tokio::test]
async fn lines_without_the_data_prefix_are_ignored() {
    let events = collected(parse_sse_events(valid_lines(&[
        "{\"a\": 1}",
        "event: message",
        ": keep-alive",
    ])))
    .await;

    assert!(events.is_empty());
}

#[tokio::test]
async fn a_broken_line_stream_is_reported() {
    let lines = stream::iter([
        Ok("data: {\"a\": 1}".to_owned()),
        Err(FixtureError("the stream breaks")),
    ]);

    let events = collected(parse_sse_events(lines)).await;

    assert_eq!(events.len(), 2);
    events[0].as_ref().expect("the first event is read");
    assert!(matches!(events[1], Err(AgentCoreError::Runtime(_))));
}

#[tokio::test]
async fn non_streaming_returns_the_agent_json() {
    let mut runtime = fixture();
    runtime.body = br#"{"jsonrpc":"2.0","id":"req-1","result":{"kind":"message"}}"#.to_vec();
    let mut handler = AgentCoreHandler {
        runtime,
        ids: Source,
    };
    let request = params(Some(json!(CONTEXT_ID)));
    let litellm = bearer_params();

    let value = handler
        .handle_non_streaming(&handler_call(&request, &litellm))
        .await
        .expect("the agent answers");

    assert_eq!(
        value,
        parsed(r#"{"jsonrpc":"2.0","id":"req-1","result":{"kind":"message"}}"#)
    );
}

#[tokio::test]
async fn an_agent_error_body_is_returned() {
    let mut runtime = fixture();
    runtime.body = br#"{"jsonrpc":"2.0","error":{"code":-32000}}"#.to_vec();
    let mut handler = AgentCoreHandler {
        runtime,
        ids: Source,
    };
    let request = params(Some(json!(CONTEXT_ID)));
    let litellm = bearer_params();

    let value = handler
        .handle_non_streaming(&handler_call(&request, &litellm))
        .await
        .expect("the agent answers");

    assert_eq!(value["error"]["code"], parsed("-32000"));
}

#[tokio::test]
async fn a_non_success_status_is_an_http_error() {
    let mut runtime = fixture();
    runtime.status = 500;
    let mut handler = AgentCoreHandler {
        runtime,
        ids: Source,
    };
    let request = params(Some(json!(CONTEXT_ID)));
    let litellm = bearer_params();

    let error = handler
        .handle_non_streaming(&handler_call(&request, &litellm))
        .await
        .expect_err("the agent failed");

    assert!(matches!(error, AgentCoreError::Status { status: 500 }));
}

#[tokio::test]
async fn a_non_json_reply_is_a_decode_error() {
    let mut runtime = fixture();
    runtime.body = b"not json".to_vec();
    let mut handler = AgentCoreHandler {
        runtime,
        ids: Source,
    };
    let request = params(Some(json!(CONTEXT_ID)));
    let litellm = bearer_params();

    let error = handler
        .handle_non_streaming(&handler_call(&request, &litellm))
        .await
        .expect_err("the agent reply is not JSON");

    assert!(matches!(
        error,
        AgentCoreError::Transform(TransformError::Decode(_))
    ));
}

#[tokio::test]
async fn a_failed_post_is_a_runtime_error() {
    let mut runtime = fixture();
    runtime.post_error = Some("the connection is refused");
    let mut handler = AgentCoreHandler {
        runtime,
        ids: Source,
    };
    let request = params(Some(json!(CONTEXT_ID)));
    let litellm = bearer_params();

    let error = handler
        .handle_non_streaming(&handler_call(&request, &litellm))
        .await
        .expect_err("the post failed");

    assert!(matches!(error, AgentCoreError::Runtime(FixtureError(_))));
}

#[tokio::test]
async fn streaming_yields_every_sse_event() {
    let mut runtime = fixture();
    runtime.content_type = "text/event-stream".to_owned();
    runtime.lines = vec![
        "data: {\"kind\": \"task\"}".to_owned(),
        "data: not-json".to_owned(),
        "data: {\"kind\": \"status\"}".to_owned(),
    ];
    let mut handler = AgentCoreHandler {
        runtime,
        ids: Source,
    };
    let request = params(Some(json!(CONTEXT_ID)));
    let litellm = bearer_params();

    let stream = handler
        .handle_streaming(&handler_call(&request, &litellm))
        .await
        .expect("the agent streams");
    let events = collected(stream).await;

    assert_eq!(
        values(events),
        vec![parsed(r#"{"kind":"task"}"#), parsed(r#"{"kind":"status"}"#)]
    );
}

#[tokio::test]
async fn a_json_content_type_yields_one_event() {
    let mut runtime = fixture();
    runtime.content_type = "application/json; charset=utf-8".to_owned();
    runtime.lines = vec!["{\"a\": 1,".to_owned(), " \"b\": 2}".to_owned()];
    let mut handler = AgentCoreHandler {
        runtime,
        ids: Source,
    };
    let request = params(Some(json!(CONTEXT_ID)));
    let litellm = bearer_params();

    let stream = handler
        .handle_streaming(&handler_call(&request, &litellm))
        .await
        .expect("the agent answers with JSON");
    let events = collected(stream).await;

    assert_eq!(values(events), vec![parsed(r#"{"a":1,"b":2}"#)]);
}

#[tokio::test]
async fn the_streaming_status_is_checked_before_the_content_type() {
    let mut runtime = fixture();
    runtime.status = 502;
    runtime.content_type = "application/json".to_owned();
    let mut handler = AgentCoreHandler {
        runtime,
        ids: Source,
    };
    let request = params(Some(json!(CONTEXT_ID)));
    let litellm = bearer_params();

    let error = handler
        .handle_streaming(&handler_call(&request, &litellm))
        .await
        .err()
        .expect("the agent failed");

    assert!(matches!(error, AgentCoreError::Status { status: 502 }));
}

#[tokio::test]
async fn missing_params_non_streaming() {
    let mut config = BedrockAgentCoreConfig {
        handler: AgentCoreHandler {
            runtime: fixture(),
            ids: Source,
        },
    };
    let request = params(None);

    let error = config
        .handle_non_streaming(&config_call(&request, None))
        .await
        .expect_err("litellm_params is required");

    assert_eq!(error.to_string(), MISSING_PARAMS);
    assert!(matches!(
        error,
        AgentCoreError::Transform(TransformError::MissingParams)
    ));
}

#[tokio::test]
async fn missing_params_streaming() {
    let mut config = BedrockAgentCoreConfig {
        handler: AgentCoreHandler {
            runtime: fixture(),
            ids: Source,
        },
    };
    let request = params(None);

    let error = config
        .handle_streaming(&config_call(&request, None))
        .await
        .err()
        .expect("litellm_params is required");

    assert_eq!(error.to_string(), MISSING_PARAMS);
}

#[tokio::test]
async fn collected_lines_are_joined_with_newlines() {
    let body = collect_body::<_, FixtureError>(valid_lines(&["first", "second"]))
        .await
        .expect("the lines are collected");

    assert_eq!(body, "first\nsecond");
}

#[rstest]
#[case::one_line_at_the_limit(vec![MAX_RESPONSE_BYTES], true)]
#[case::one_line_over_the_limit(vec![MAX_RESPONSE_BYTES + 1], false)]
#[case::two_lines_at_the_limit(vec![MAX_RESPONSE_BYTES - 2, 1], true)]
#[case::the_separator_counts(vec![MAX_RESPONSE_BYTES - 1, 1], false)]
#[tokio::test]
async fn the_collected_reply_is_capped(#[case] lengths: Vec<usize>, #[case] accepted: bool) {
    let lines = stream::iter(
        lengths
            .into_iter()
            .map(|length| Ok::<_, FixtureError>("a".repeat(length))),
    );

    let result = collect_body(lines).await;

    if accepted {
        assert_eq!(result.expect("the reply fits").len(), MAX_RESPONSE_BYTES);
    } else {
        assert!(matches!(
            result,
            Err(AgentCoreError::ResponseTooLarge { max }) if max == MAX_RESPONSE_BYTES
        ));
    }
}

#[test]
fn optional_params_leave_out_the_model_and_provider() {
    let params = litellm_params(&[
        ("model", json!("bedrock/agentcore/arn")),
        ("custom_llm_provider", json!("bedrock")),
        ("api_key", json!("jwt-token")),
        ("runtimeSessionId", json!("abc")),
    ]);

    let names: Vec<&str> = params.optional.keys().map(String::as_str).collect();

    assert_eq!(names, vec!["api_key", "runtimeSessionId"]);
}

#[tokio::test]
async fn the_principal_in_litellm_params_scopes_the_session() {
    let runtime = fixture();
    let request = params(Some(json!(CONTEXT_ID)));
    let litellm = litellm_params(&[
        ("model", json!("bedrock/agentcore/arn")),
        ("api_key", json!("jwt-token")),
        ("litellm_a2a_user_api_key_hash", json!("key-2")),
    ]);
    let expected_session = format!("{SCOPED_PREFIX}-{CONTEXT_ID}");

    let result = signed_request(&runtime, &request, &litellm, RpcMethod::Send).await;

    assert_eq!(
        result
            .expect("the request is signed")
            .headers
            .get(SESSION_HEADER)
            .map(String::as_str),
        Some(expected_session.as_str())
    );
}

#[tokio::test]
async fn an_empty_params_map_is_missing_params() {
    let mut config = BedrockAgentCoreConfig {
        handler: AgentCoreHandler {
            runtime: fixture(),
            ids: Source,
        },
    };
    let request = params(None);
    let raw = BTreeMap::new();

    let error = config
        .handle_non_streaming(&config_call(&request, Some(&raw)))
        .await
        .expect_err("an empty litellm_params is as good as none");

    assert_eq!(error.to_string(), MISSING_PARAMS);
}

#[tokio::test]
async fn the_config_delegates_with_present_params() {
    let mut runtime = fixture();
    runtime.body = br#"{"jsonrpc":"2.0","id":"req-1","result":{"kind":"message"}}"#.to_vec();
    let mut config = BedrockAgentCoreConfig {
        handler: AgentCoreHandler {
            runtime,
            ids: Source,
        },
    };
    let request = params(None);
    let raw = BTreeMap::from([
        ("model".to_owned(), json!("bedrock/agentcore/arn")),
        ("api_key".to_owned(), json!("jwt-token")),
    ]);

    let value = config
        .handle_non_streaming(&config_call(&request, Some(&raw)))
        .await
        .expect("the agent answers");

    assert_eq!(value["result"]["kind"], json!("message"));
}

#[test]
fn a_non_text_model_is_an_error_naming_the_field() {
    let raw = BTreeMap::from([("model".to_owned(), parsed("5"))]);

    let error = LitellmParams::parse(&raw)
        .err()
        .expect("the model must be a string");

    assert!(matches!(
        error,
        TransformError::FieldType { field: "model" }
    ));
}

#[tokio::test]
async fn the_signed_request_is_what_gets_posted() {
    let mut runtime = fixture();
    runtime.body = br#"{"jsonrpc":"2.0","id":"req-1","result":{}}"#.to_vec();
    let mut handler = AgentCoreHandler {
        runtime,
        ids: Source,
    };
    let request = params(Some(json!(CONTEXT_ID)));
    let litellm = bearer_params();

    handler
        .handle_non_streaming(&handler_call(&request, &litellm))
        .await
        .expect("the agent answers");

    let posted = handler.runtime.posted.lock().expect("the fixture lock");
    let sent: AgentJson = serde_json::from_slice(&posted[0].body).expect("the body is JSON");
    assert_eq!(sent["method"], json!("message/send"));
    assert_eq!(posted.len(), 1);
    assert_eq!(posted[0].url, format!("{URL}agentcore/arn"));
    assert_eq!(
        posted[0].headers.get(SESSION_HEADER).map(String::as_str),
        Some(format!("{SCOPED_PREFIX}-{CONTEXT_ID}").as_str())
    );
}

#[tokio::test]
async fn the_streaming_post_carries_the_stream_envelope() {
    let mut runtime = fixture();
    runtime.content_type = "text/event-stream".to_owned();
    let mut handler = AgentCoreHandler {
        runtime,
        ids: Source,
    };
    let request = params(Some(json!(CONTEXT_ID)));
    let litellm = bearer_params();

    let _events = handler
        .handle_streaming(&handler_call(&request, &litellm))
        .await
        .expect("the agent streams");

    let posted = handler.runtime.posted.lock().expect("the fixture lock");
    let sent: AgentJson = serde_json::from_slice(&posted[0].body).expect("the body is JSON");
    assert_eq!(sent["method"], json!("message/stream"));
    assert_eq!(posted[0].url, format!("{URL}agentcore/arn"));
}

#[tokio::test]
async fn the_config_streams_with_present_params() {
    let mut runtime = fixture();
    runtime.content_type = "text/event-stream".to_owned();
    runtime.lines = vec!["data: {\"kind\": \"task\"}".to_owned()];
    let mut config = BedrockAgentCoreConfig {
        handler: AgentCoreHandler {
            runtime,
            ids: Source,
        },
    };
    let request = params(None);
    let raw = BTreeMap::from([
        ("model".to_owned(), json!("bedrock/agentcore/arn")),
        ("api_key".to_owned(), json!("jwt-token")),
    ]);

    let stream = config
        .handle_streaming(&config_call(&request, Some(&raw)))
        .await
        .expect("the agent streams");
    let events = collected(stream).await;

    assert_eq!(values(events), vec![parsed(r#"{"kind":"task"}"#)]);
}
