#![cfg(test)]

mod support;

use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_core::dispatch::error::DispatchError;
use betterlite_core::environment_validation::probe::{ModelProbe, ProbeCompletion};
use betterlite_core::responses_bridge::ResponsesBridgeCapabilities;
use betterlite_error::error::ErrorClass;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

use crate::call_setup::correlation::CallOutcome;
use crate::call_wrapper::error::{class_name, class_status};
use crate::completion_call::bridge_facts::ModelBridgeFacts;
use crate::completion_call::error::CompletionCallError;
use crate::completion_call::fallbacks::run_fallbacks;
use crate::completion_call::probe::{CompletionProbe, ProbeFailure};
use crate::completion_call::retry::{RetryStrategy, with_retries};
use crate::completion_call::tokens::{
    LOCAL_TOKENIZER, TokenCountRequest, TokenCountResponse, count_tokens,
};
use crate::completion_request::args::JsonObject;
use crate::completion_request::call::{
    FallbackCall, MockCall, ModelListCall, PreparedCall, PreparedCompletion,
};
use support::{
    FakeCallback, FakeFacts, FakeFoundry, FakeLocalCounter, FakeProviderCounter, Harness,
    RecordingTarget, args_from, chat_body, dig, json, request, secret, text_of,
};

const USER: &str = r#"{"role":"user","content":"hi"}"#;

fn body_text(outcome: &CallOutcome) -> Option<&Json> {
    match outcome {
        CallOutcome::Response(answer) => Some(answer),
        CallOutcome::ChatStream(_) | CallOutcome::ResponsesStream(_) => None,
    }
}

fn answer_choices(answer: &Json) -> &[Json] {
    let Json::Object(fields) = answer else {
        panic!("the answer is an object");
    };
    let Some(Json::List(choices)) = fields.get("choices") else {
        panic!("the answer carries choices");
    };
    choices
}

fn zero_retries(value: Option<&Json>) -> bool {
    matches!(value, Some(Json::Number(number)) if number.as_i64() == Some(0))
}

// ---- target_ ----

#[tokio::test]
async fn target_dispatch_returns_the_finished_response() {
    let harness = Harness::new();
    let target = harness.target();
    let call = request(&format!(r#"{{"model":"gpt-4o","messages":[{USER}]}}"#));
    let outcome = target.call(call, None).await.unwrap();
    let Some(answer) = body_text(&outcome) else {
        panic!("a plain call answers a finished response");
    };
    let choices = answer_choices(answer);
    assert_eq!(
        dig(&choices[0], &["message", "content"]),
        Some(&Json::Text(String::from("hello")))
    );
    let seen = harness.chat.seen();
    assert_eq!(seen.len(), 1);
    assert!(!seen[0].stream);
    assert_eq!(seen[0].model.as_deref(), Some("gpt-4o"));
}

#[tokio::test]
async fn target_stream_call_returns_the_chat_stream() {
    let harness = Harness::new();
    harness.chat.streams(&[
        "data: {\"id\":\"chunk-1\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"he\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"chunk-2\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"llo\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    ]);
    let target = harness.target();
    let call = request(&format!(
        r#"{{"model":"gpt-4o","messages":[{USER}],"stream":true}}"#
    ));
    let outcome = target.call(call, None).await.unwrap();
    let CallOutcome::ChatStream(stream) = outcome else {
        panic!("a streaming call answers a chat stream");
    };
    assert!(!stream.converted_from_mock);
    let Json::List(chunks) = &stream.model_response else {
        panic!("the stream holds its chunks");
    };
    assert_eq!(chunks.len(), 2);
    assert!(
        dig(&chunks[1], &["choices"]).is_some(),
        "the second chunk is the provider's"
    );
    assert!(harness.chat.seen()[0].stream);
}

#[tokio::test]
async fn target_unknown_model_fails_as_a_bad_request() {
    let harness = Harness::new();
    let target = harness.target();
    let call = request(&format!(
        r#"{{"model":"missing-model","messages":[{USER}]}}"#
    ));
    let error = target.call(call, None).await.unwrap_err();
    assert_eq!(class_name(&error.class), "BadRequestError");
    assert_eq!(class_status(&error.class), Some(400));
}

#[tokio::test]
async fn target_model_list_skips_a_failed_deployment() {
    let harness = Harness::new();
    let target = harness.target();
    let deployments = vec![
        JsonObject::from([(
            String::from("model"),
            Json::Text(String::from("missing-model")),
        )]),
        JsonObject::from([(String::from("model"), Json::Text(String::from("gpt-4o")))]),
    ];
    let call = ModelListCall {
        deployments,
        args: args_from(&format!(r#"{{"model":"gpt-4o","messages":[{USER}]}}"#)),
    };
    let outcome = target
        .run(PreparedCompletion::ModelList(call), false, None)
        .await
        .unwrap();
    let Some(answer) = body_text(&outcome) else {
        panic!("the second deployment answers");
    };
    assert_eq!(
        dig(answer, &["id"]),
        Some(&Json::Text(String::from("chatcmpl-1")))
    );
}

#[tokio::test]
async fn target_model_list_without_an_answer_names_the_failure() {
    let harness = Harness::new();
    let target = harness.target();
    let deployments = vec![JsonObject::from([(
        String::from("model"),
        Json::Text(String::from("missing-model")),
    )])];
    let call = ModelListCall {
        deployments,
        args: args_from(&format!(r#"{{"model":"gpt-4o","messages":[{USER}]}}"#)),
    };
    let error = target
        .run(PreparedCompletion::ModelList(call), false, None)
        .await
        .unwrap_err();
    assert!(matches!(&error, CompletionCallError::Call(_)));
    assert!(error.to_string().contains("missing-model"));
}

#[tokio::test]
async fn target_mock_answers_from_the_mock_response() {
    let harness = Harness::new();
    let target = harness.target();
    let mut kwargs = JsonObject::new();
    kwargs.insert(
        String::from("mock_response"),
        Json::Text(String::from("hi")),
    );
    let call = MockCall {
        call: PreparedCall {
            model: String::from("gpt-4o"),
            kwargs,
            ..PreparedCall::default()
        },
        acompletion: false,
        mock_delay: None,
        prompt_tokens: None,
    };
    let outcome = target
        .run(PreparedCompletion::Mock(call), false, None)
        .await
        .unwrap();
    let Some(answer) = body_text(&outcome) else {
        panic!("a mock answers a finished response");
    };
    let choices = answer_choices(answer);
    assert_eq!(
        dig(&choices[0], &["message", "content"]),
        Some(&Json::Text(String::from("hi")))
    );
    assert_eq!(
        dig(answer, &["id"]),
        Some(&Json::Text(String::from("chatcmpl-call-1")))
    );
    assert!(matches!(
        dig(answer, &["created"]),
        Some(Json::Number(number)) if number.as_u64() == Some(1_700_000_000)
    ));
}

#[tokio::test]
async fn target_mock_stream_is_a_chat_stream() {
    let harness = Harness::new();
    let target = harness.target();
    let mut kwargs = JsonObject::new();
    kwargs.insert(
        String::from("mock_response"),
        Json::Text(String::from("hello")),
    );
    let call = MockCall {
        call: PreparedCall {
            model: String::from("gpt-4o"),
            kwargs,
            stream: true,
            ..PreparedCall::default()
        },
        acompletion: true,
        mock_delay: None,
        prompt_tokens: None,
    };
    let outcome = target
        .run(PreparedCompletion::Mock(call), true, None)
        .await
        .unwrap();
    let CallOutcome::ChatStream(stream) = outcome else {
        panic!("a streaming mock answers a chat stream");
    };
    assert!(stream.converted_from_mock);
    let Json::List(chunks) = &stream.model_response else {
        panic!("the mock stream holds its chunks");
    };
    assert_eq!(chunks.len(), 2, "five characters in three-character deltas");
}

#[tokio::test]
async fn target_palm_raises_its_value_error() {
    let harness = Harness::new();
    let target = harness.target();
    let call = PreparedCall {
        model: String::from("palm"),
        custom_llm_provider: Some(String::from("palm")),
        ..PreparedCall::default()
    };
    let error = target
        .run(PreparedCompletion::Dispatch(call), false, None)
        .await
        .unwrap_err();
    assert!(matches!(&error, CompletionCallError::Palm));
    let mapped = target.error(error, "palm");
    assert!(
        mapped
            .to_string()
            .contains("Palm was decommissioned on October 2024")
    );
}

#[tokio::test]
async fn target_mcp_loop_without_a_runner_fails_by_name() {
    let harness = Harness::new();
    let target = harness.target();
    let error = target
        .run(
            PreparedCompletion::McpLoop(PreparedCall::default()),
            false,
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(&error, CompletionCallError::NoMcpLoop));
}

#[tokio::test]
async fn target_responses_bridge_reaches_the_responses_entry() {
    let harness = Harness::new();
    let target = harness.target();
    let error = target
        .run(
            PreparedCompletion::ResponsesBridge(PreparedCall::default()),
            false,
            None,
        )
        .await
        .unwrap_err();
    let CompletionCallError::Dispatch(DispatchError::NotBuilt { entry }) = error else {
        panic!("the bridge goes to the responses entry");
    };
    assert_eq!(entry, "responses");
}

// ---- fallback_ ----

#[tokio::test]
async fn fallback_tries_the_original_each_entry_and_keeps_overrides() {
    let target = RecordingTarget::failing_then_ok(1);
    let args = args_from(&format!(r#"{{"model":"gpt-4o","messages":[{USER}]}}"#));
    let call = FallbackCall {
        fallbacks: vec![
            json(r#""gpt-3.5-turbo""#),
            json(r#"{"model":"claude","api_base":"https://claude.test"}"#),
        ],
        args,
    };
    let outcome = run_fallbacks(&target, call, None).await.unwrap();
    let calls = target.call_map();
    assert_eq!(calls.len(), 2);
    assert_eq!(text_of(calls[0].get("model")), Some("gpt-4o"));
    assert_eq!(text_of(calls[1].get("model")), Some("claude"));
    assert_eq!(
        text_of(calls[1].get("api_base")),
        Some("https://claude.test")
    );
    assert_eq!(
        text_of(calls[0].get("litellm_call_id")),
        text_of(calls[1].get("litellm_call_id"))
    );
    assert!(text_of(calls[0].get("litellm_call_id")).is_some());
    let Some(answer) = body_text(&outcome) else {
        panic!("the second attempt answers");
    };
    let attempt = dig(
        answer,
        &[
            "_hidden_params",
            "additional_headers",
            "x-litellm-attempted-fallbacks",
        ],
    );
    assert!(matches!(attempt, Some(Json::Number(number)) if number.as_u64() == Some(1)));
}

#[tokio::test]
async fn fallback_dict_without_a_model_keeps_the_original() {
    let target = RecordingTarget::failing_then_ok(1);
    let args = args_from(&format!(r#"{{"model":"gpt-4o","messages":[{USER}]}}"#));
    let call = FallbackCall {
        fallbacks: vec![json(r#"{"temperature":0.2}"#)],
        args,
    };
    run_fallbacks(&target, call, None).await.unwrap();
    let calls = target.call_map();
    assert_eq!(text_of(calls[1].get("model")), Some("gpt-4o"));
    assert!(matches!(
        calls[1].get("temperature"),
        Some(Json::Number(number)) if number.as_f64() == Some(0.2)
    ));
}

#[tokio::test]
async fn fallback_internal_params_never_reach_an_attempt() {
    let target = RecordingTarget::failing_then_ok(1);
    let mut args = args_from(&format!(r#"{{"model":"gpt-4o","messages":[{USER}]}}"#));
    for name in [
        "skip_mcp_handler",
        "mcp_handler_context",
        "_skip_mcp_handler",
    ] {
        args.kwargs.insert(String::from(name), Json::Bool(true));
    }
    let call = FallbackCall {
        fallbacks: vec![json(r#""gpt-3.5-turbo""#)],
        args,
    };
    run_fallbacks(&target, call, None).await.unwrap();
    for attempt in target.call_map() {
        for name in [
            "skip_mcp_handler",
            "mcp_handler_context",
            "_skip_mcp_handler",
        ] {
            assert!(!attempt.contains_key(name), "{name} stays internal");
        }
        assert!(!attempt.contains_key("fallbacks"));
        assert!(!attempt.contains_key("acompletion"));
    }
}

#[tokio::test]
async fn fallback_all_fail_reports_litellms_text() {
    let target = RecordingTarget::always_failing();
    let args = args_from(&format!(r#"{{"model":"gpt-4o","messages":[{USER}]}}"#));
    let call = FallbackCall {
        fallbacks: vec![json(r#""gpt-3.5-turbo""#)],
        args,
    };
    let error = run_fallbacks(&target, call, None).await.unwrap_err();
    let CompletionCallError::AllFallbacksFailed { last } = error else {
        panic!("every attempt failed");
    };
    assert_eq!(last, "first down");
    assert_eq!(
        CompletionCallError::AllFallbacksFailed { last }.to_string(),
        "first down. All fallback attempts failed. Enable verbose logging with `litellm.set_verbose=True` for details."
    );
}

// ---- async_ ----

#[tokio::test]
async fn async_mock_delay_sleeps_through_the_injected_sleeper() {
    let harness = Harness::new();
    let target = harness.async_target();
    let call = request(&format!(
        r#"{{"model":"gpt-4o","messages":[{USER}],"mock_response":"hi","mock_delay":0.5}}"#
    ));
    let outcome = target.call(call, None).await.unwrap();
    assert_eq!(harness.sleeper.slept(), vec![Duration::from_millis(500)]);
    let Some(answer) = body_text(&outcome) else {
        panic!("the delayed mock answers");
    };
    let choices = answer_choices(answer);
    assert_eq!(
        dig(&choices[0], &["message", "content"]),
        Some(&Json::Text(String::from("hi")))
    );
}

#[tokio::test]
async fn async_text_completion_reply_becomes_a_chat_reply() {
    let harness = Harness::new();
    let target = harness.async_target();
    let reply = json(
        r#"{"id":"cmpl-1","object":"text_completion","created":1,"model":"gpt-3.5-turbo-instruct","choices":[{"text":"hello","index":0,"finish_reason":"stop"}]}"#,
    );
    let converted = target
        .convert_text_reply(Some("text-completion-openai"), CallOutcome::Response(reply))
        .unwrap();
    let Some(answer) = body_text(&converted) else {
        panic!("the converted answer is a response");
    };
    assert_eq!(
        dig(answer, &["object"]),
        Some(&Json::Text(String::from("chat.completion")))
    );
    let choices = answer_choices(answer);
    assert_eq!(
        dig(&choices[0], &["message", "content"]),
        Some(&Json::Text(String::from("hello")))
    );
    assert_eq!(
        dig(answer, &["model"]),
        Some(&Json::Text(String::from("gpt-3.5-turbo-instruct")))
    );
}

#[tokio::test]
async fn async_other_providers_keep_their_reply() {
    let harness = Harness::new();
    let target = harness.async_target();
    let reply =
        json(r#"{"id":"cmpl-1","object":"text_completion","created":1,"model":"m","choices":[]}"#);
    let outcome = target
        .convert_text_reply(Some("openai"), CallOutcome::Response(reply))
        .unwrap();
    let Some(answer) = body_text(&outcome) else {
        panic!("the reply is a response");
    };
    assert_eq!(
        dig(answer, &["object"]),
        Some(&Json::Text(String::from("text_completion")))
    );
}

#[tokio::test]
async fn async_agentic_loop_replaces_the_response() {
    let mut harness = Harness::new();
    harness.callbacks.push(Arc::new(FakeCallback {
        reply: chat_body("looped"),
    }));
    let target = harness.async_target();
    let call = request(&format!(r#"{{"model":"gpt-4o","messages":[{USER}]}}"#));
    let outcome = target.call(call, None).await.unwrap();
    let Some(answer) = body_text(&outcome) else {
        panic!("the loop answers a response");
    };
    assert_eq!(
        dig(answer, &["id"]),
        Some(&Json::Text(String::from("chatcmpl-agentic")))
    );
    let choices = answer_choices(answer);
    assert_eq!(
        dig(&choices[0], &["message", "content"]),
        Some(&Json::Text(String::from("looped")))
    );
}

#[tokio::test]
async fn async_error_defaults_the_provider_to_openai() {
    let harness = Harness::new();
    let target = harness.async_target();
    let call = request(&format!(
        r#"{{"model":"missing-model","messages":[{USER}]}}"#
    ));
    let error = target.call(call, None).await.unwrap_err();
    let ErrorClass::BadRequest(details) = &error.class else {
        panic!("a missing deployment is a bad request");
    };
    assert_eq!(details.provider.as_deref(), Some("openai"));
}

#[test]
fn async_fallbacks_with_no_answer_uses_litellms_message() {
    assert_eq!(
        CompletionCallError::NoFallbackResponse.to_string(),
        "No response from fallbacks. Got none. Turn on `litellm.set_verbose=True` to see more details."
    );
}

// ---- retry_ ----

#[tokio::test]
async fn retry_defaults_to_three_attempts_and_zeroes_inner_retries() {
    let target = RecordingTarget::always_failing();
    let sleeper = support::FakeSleeper::default();
    let call = request("{}");
    let error = with_retries(&target, call, None, &sleeper)
        .await
        .unwrap_err();
    assert_eq!(target.call_map().len(), 3);
    for attempt in target.call_map() {
        assert!(zero_retries(attempt.get("num_retries")));
        assert!(zero_retries(attempt.get("max_retries")));
    }
    assert_eq!(error.to_string(), "first down");
    assert!(sleeper.slept().is_empty(), "constant retry waits nothing");
}

#[tokio::test]
async fn retry_exponential_waits_double_and_cap_at_ten_seconds() {
    let target = RecordingTarget::always_failing();
    let sleeper = support::FakeSleeper::default();
    let call = request(r#"{"num_retries":7,"retry_strategy":"exponential_backoff_retry"}"#);
    with_retries(&target, call, None, &sleeper)
        .await
        .unwrap_err();
    assert_eq!(target.call_map().len(), 7);
    assert_eq!(
        sleeper.slept(),
        vec![
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4),
            Duration::from_secs(8),
            Duration::from_secs(10),
            Duration::from_secs(10),
        ]
    );
}

#[tokio::test]
async fn retry_stops_on_the_first_answer() {
    let target = RecordingTarget::failing_then_ok(1);
    let sleeper = support::FakeSleeper::default();
    let call = request(r#"{"num_retries":5,"retry_strategy":"exponential_backoff_retry"}"#);
    let outcome = with_retries(&target, call, None, &sleeper).await.unwrap();
    assert_eq!(target.call_map().len(), 2);
    assert!(body_text(&outcome).is_some());
    assert_eq!(sleeper.slept(), vec![Duration::from_secs(1)]);
}

#[tokio::test]
async fn retry_one_means_a_single_call() {
    let target = RecordingTarget::always_failing();
    let sleeper = support::FakeSleeper::default();
    let call = request(r#"{"num_retries":1}"#);
    let error = with_retries(&target, call, None, &sleeper)
        .await
        .unwrap_err();
    assert_eq!(target.call_map().len(), 1);
    assert_eq!(error.to_string(), "first down");
}

#[test]
fn retry_strategy_rejects_a_non_text_value() {
    let error =
        RetryStrategy::parse(Some(&Json::Number(serde_json::Number::from(5_i64)))).unwrap_err();
    assert!(error.to_string().contains("retry_strategy"));
    assert_eq!(
        RetryStrategy::parse(Some(&Json::Text(String::from("constant_retry")))).unwrap(),
        RetryStrategy::Constant
    );
}

// ---- probe_ ----

async fn read_request(stream: &mut TcpStream) -> String {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        let read = stream.read(&mut chunk).await.unwrap();
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8_lossy(&buffer).into_owned()
}

async fn spawn_server(status: u16) -> (String, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let (mut stream, _peer) = listener.accept().await.unwrap();
        let seen = read_request(&mut stream).await;
        let answer = format!("HTTP/1.1 {status} X\r\ncontent-length: 0\r\n\r\n");
        stream.write_all(answer.as_bytes()).await.unwrap();
        seen
    });
    (format!("http://{address}/v1"), handle)
}

fn client() -> ProxiedClient {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &std::collections::BTreeMap::new()).unwrap(),
    );
    ProxiedClient::outbound(&policy).unwrap()
}

#[tokio::test]
async fn probe_lookup_sends_a_bearer_get_for_the_model() {
    let harness = Harness::new();
    let target = harness.target();
    let client = client();
    let (base, server) = spawn_server(200).await;
    let probe = CompletionProbe {
        client: &client,
        completion: &target,
        api_base: Some(base),
        api_key: Some(secret("test-key")),
        timeout: Duration::from_secs(5),
    };
    probe.retrieve_openai_model("gpt/4o").await.unwrap();
    let seen = server.await.unwrap();
    assert!(seen.contains("GET /v1/models/gpt%2F4o HTTP/1.1"), "{seen}");
    assert!(
        seen.to_ascii_lowercase()
            .contains("authorization: bearer test-key"),
        "{seen}"
    );
}

#[tokio::test]
async fn probe_lookup_reports_the_answer_status() {
    let harness = Harness::new();
    let target = harness.target();
    let client = client();
    let (base, server) = spawn_server(404).await;
    let probe = CompletionProbe {
        client: &client,
        completion: &target,
        api_base: Some(base),
        api_key: Some(secret("test-key")),
        timeout: Duration::from_secs(5),
    };
    let error = probe.retrieve_openai_model("gpt-4o").await.unwrap_err();
    assert!(matches!(&error, ProbeFailure::Status { status: 404 }));
    server.await.unwrap();
}

#[tokio::test]
async fn probe_lookup_without_a_base_fails() {
    let harness = Harness::new();
    let target = harness.target();
    let client = client();
    let probe = CompletionProbe {
        client: &client,
        completion: &target,
        api_base: None,
        api_key: None,
        timeout: Duration::from_secs(5),
    };
    let error = probe.retrieve_openai_model("gpt-4o").await.unwrap_err();
    assert!(matches!(&error, ProbeFailure::MissingApiBase));
}

#[tokio::test]
async fn probe_complete_calls_the_completion_target() {
    let harness = Harness::new();
    let target = harness.target();
    let client = client();
    let key = secret("probe-key");
    let probe = CompletionProbe {
        client: &client,
        completion: &target,
        api_base: None,
        api_key: None,
        timeout: Duration::from_secs(5),
    };
    let completion = ProbeCompletion {
        model: "gpt-4o",
        user_message: "Hello World",
        api_key: Some(&key),
        max_tokens: Some(10),
    };
    probe.complete(&completion).await.unwrap();
    let seen = harness.chat.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].model.as_deref(), Some("gpt-4o"));
    assert_eq!(seen[0].max_tokens, Some(10));
    assert!(seen[0].body.contains(r#""content":"Hello World""#));
    assert!(seen[0].body.contains("probe-key"));
}

// ---- bridge_facts_ ----

fn bridge<'a>(facts: &'a FakeFacts, foundry: &'a FakeFoundry) -> ModelBridgeFacts<'a, FakeFoundry> {
    ModelBridgeFacts {
        facts,
        foundry,
        openai: betterlite_config::sections::openai_credentials::Settings::default(),
        litellm_api_base: None,
    }
}

#[test]
fn bridge_facts_answers_model_info_and_unknown_models() {
    let info: Object = serde_json::from_str(r#"{"mode":"responses"}"#).unwrap();
    let facts = FakeFacts {
        info: Some(info.clone()),
        ..FakeFacts::default()
    };
    let foundry = FakeFoundry::default();
    let facts_bridge = bridge(&facts, &foundry);
    assert_eq!(facts_bridge.model_info("gpt-5", "openai").unwrap(), info);
    assert!(matches!(
        facts_bridge.model_info("other", "openai"),
        Err(betterlite_core::responses_bridge::ModelInfoError::Unknown { model }) if model == "other"
    ));
    assert_eq!(
        support::lock(&facts.seen).clone(),
        vec![String::from("gpt-5")]
    );
}

#[test]
fn bridge_facts_passes_the_foundry_questions_to_its_field() {
    let facts = FakeFacts::default();
    let foundry = FakeFoundry {
        supports: true,
        rejects: true,
        ..FakeFoundry::default()
    };
    let facts_bridge = bridge(&facts, &foundry);
    assert!(
        facts_bridge.foundry_supports_responses("azure/gpt-5", Some("https://x.openai.azure.com"))
    );
    let effort = betterlite_core::responses_bridge::ReasoningEffort::Text(String::from("high"));
    assert!(facts_bridge.foundry_chat_rejects_tools("azure/gpt-5", Some(&effort)));
    let calls = support::lock(&foundry.calls).clone();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].model, "azure/gpt-5");
    assert_eq!(
        calls[0].api_base.as_deref(),
        Some("https://x.openai.azure.com")
    );
    assert!(calls[1].rejects);
}

#[test]
fn bridge_facts_resolves_the_openai_base_in_litellms_order() {
    let facts = FakeFacts::default();
    let foundry = FakeFoundry::default();
    let mut facts_bridge = bridge(&facts, &foundry);
    assert_eq!(
        facts_bridge.resolved_openai_api_base(None),
        "https://api.openai.com/v1"
    );
    assert_eq!(
        facts_bridge.resolved_openai_api_base(Some("https://proxy.test/v1")),
        "https://proxy.test/v1"
    );
    facts_bridge.litellm_api_base = Some(String::from("https://litellm.test/v1"));
    assert_eq!(
        facts_bridge.resolved_openai_api_base(None),
        "https://litellm.test/v1"
    );
    assert_eq!(
        facts_bridge.resolved_openai_api_base(Some("https://explicit.test/v1")),
        "https://explicit.test/v1"
    );
}

#[test]
fn bridge_facts_gpt_5_check_excludes_other_providers() {
    let facts = FakeFacts::default();
    let foundry = FakeFoundry::default();
    let facts_bridge = bridge(&facts, &foundry);
    assert!(facts_bridge.is_gpt_5_model("gpt-5"));
    assert!(facts_bridge.is_gpt_5_model("openai/gpt-5"));
    assert!(facts_bridge.is_gpt_5_model("gpt-6"));
    assert!(!facts_bridge.is_gpt_5_model("azure/gpt-5"));
    assert!(!facts_bridge.is_gpt_5_model("perplexity/openai/gpt-5.2"));
    assert!(!facts_bridge.is_gpt_5_model("gpt-5-chat-latest"));
    assert!(facts_bridge.is_gpt_5_search_model("gpt-5-search-api"));
    assert!(!facts_bridge.is_gpt_5_search_model("gpt-4o"));
    assert!(facts_bridge.is_gpt_5_4_plus_model("gpt-5.4"));
    assert!(!facts_bridge.is_gpt_5_4_plus_model("gpt-5.2"));
}

// ---- tokens_ ----

fn token_messages() -> Vec<Json> {
    vec![json(USER)]
}

#[tokio::test]
async fn tokens_uses_the_provider_counter_when_it_answers() {
    let mut metadata = support::FakeMetadata::default();
    metadata.resolved_model = Some(String::from("gpt-4o-2024"));
    let mut answer = TokenCountResponse::local(11, "gpt-4o", "gpt-4o-provider");
    answer.tokenizer_type = String::from("provider_tokenizer");
    let counter = FakeProviderCounter::answering(answer);
    let local = FakeLocalCounter::new(99);
    let messages = token_messages();
    let token_request = TokenCountRequest {
        model: "gpt-4o",
        messages: Some(&messages),
        tools: None,
        system: None,
        api_key: None,
        api_base: None,
    };
    let response = count_tokens(&token_request, &metadata, Some(&counter), &local)
        .await
        .unwrap();
    assert_eq!(response.total_tokens, 11);
    assert_eq!(response.tokenizer_type, "provider_tokenizer");
    assert_eq!(counter.seen().len(), 1);
    assert!(local.seen().is_empty());
}

#[tokio::test]
async fn tokens_provider_error_falls_back_to_the_local_count() {
    let mut metadata = support::FakeMetadata::default();
    metadata.resolved_model = Some(String::from("gpt-4o-2024"));
    let mut answer = TokenCountResponse::local(11, "gpt-4o", "gpt-4o-provider");
    answer.error = true;
    let counter = FakeProviderCounter::answering(answer);
    let local = FakeLocalCounter::new(7);
    let messages = token_messages();
    let token_request = TokenCountRequest {
        model: "gpt-4o",
        messages: Some(&messages),
        tools: None,
        system: None,
        api_key: None,
        api_base: None,
    };
    let response = count_tokens(&token_request, &metadata, Some(&counter), &local)
        .await
        .unwrap();
    assert_eq!(response.total_tokens, 7);
    assert_eq!(response.tokenizer_type, LOCAL_TOKENIZER);
    assert_eq!(response.request_model, "gpt-4o");
    assert_eq!(response.model_used, "gpt-4o-2024");
    assert_eq!(local.seen().len(), 1);
}

#[tokio::test]
async fn tokens_skips_a_counter_that_does_not_serve_the_provider() {
    let metadata = support::FakeMetadata::default();
    let counter = FakeProviderCounter::unavailable();
    let local = FakeLocalCounter::new(3);
    let messages = token_messages();
    let token_request = TokenCountRequest {
        model: "gpt-4o",
        messages: Some(&messages),
        tools: None,
        system: None,
        api_key: None,
        api_base: None,
    };
    let response = count_tokens(&token_request, &metadata, Some(&counter), &local)
        .await
        .unwrap();
    assert_eq!(response.total_tokens, 3);
    assert!(counter.seen().is_empty());
    assert_eq!(local.seen().len(), 1);
}

#[tokio::test]
async fn tokens_system_message_is_prepended_only_with_messages() {
    let metadata = support::FakeMetadata::default();
    let local = FakeLocalCounter::new(5);
    let messages = token_messages();
    let with_messages = TokenCountRequest {
        model: "gpt-4o",
        messages: Some(&messages),
        tools: None,
        system: Some("rules"),
        api_key: None,
        api_base: None,
    };
    count_tokens(&with_messages, &metadata, None, &local)
        .await
        .unwrap();
    let seen = local.seen();
    assert_eq!(seen[0].len(), 2);
    assert_eq!(
        dig(&seen[0][0], &["role"]),
        Some(&Json::Text(String::from("system")))
    );
    let empty: [Json; 0] = [];
    let without_messages = TokenCountRequest {
        model: "gpt-4o",
        messages: Some(&empty),
        tools: None,
        system: Some("rules"),
        api_key: None,
        api_base: None,
    };
    count_tokens(&without_messages, &metadata, None, &local)
        .await
        .unwrap();
    assert!(local.seen()[1].is_empty());
}

#[tokio::test]
async fn tokens_explicit_key_and_base_win_over_the_resolution() {
    let mut metadata = support::FakeMetadata::default();
    metadata.resolved_base = Some(String::from("https://resolved.test/v1"));
    metadata.dynamic_key = Some(String::from("resolved-key"));
    let counter = FakeProviderCounter::answering(TokenCountResponse::local(4, "gpt-4o", "m"));
    let local = FakeLocalCounter::new(0);
    let key = secret("explicit-key");
    let messages = token_messages();
    let token_request = TokenCountRequest {
        model: "gpt-4o",
        messages: Some(&messages),
        tools: None,
        system: None,
        api_key: Some(&key),
        api_base: Some("https://explicit.test/v1"),
    };
    count_tokens(&token_request, &metadata, Some(&counter), &local)
        .await
        .unwrap();
    let seen = counter.seen();
    assert_eq!(
        seen[0].api_base.as_deref(),
        Some("https://explicit.test/v1")
    );
    assert_eq!(seen[0].key.as_deref(), Some("explicit-key"));
}

#[tokio::test]
async fn tokens_absent_provider_answer_falls_back_to_the_local_count() {
    let metadata = support::FakeMetadata::default();
    let counter = FakeProviderCounter::silent();
    let local = FakeLocalCounter::new(9);
    let messages = token_messages();
    let token_request = TokenCountRequest {
        model: "gpt-4o",
        messages: Some(&messages),
        tools: None,
        system: None,
        api_key: None,
        api_base: None,
    };
    let response = count_tokens(&token_request, &metadata, Some(&counter), &local)
        .await
        .unwrap();
    assert_eq!(response.total_tokens, 9);
    assert_eq!(response.tokenizer_type, LOCAL_TOKENIZER);
}

#[test]
fn tokens_local_answer_carries_litellms_shape() {
    let response = TokenCountResponse::local(12, "gpt-4o", "gpt-4o-2024");
    assert_eq!(response.total_tokens, 12);
    assert_eq!(response.tokenizer_type, "local_tokenizer");
    assert!(!response.error);
}
