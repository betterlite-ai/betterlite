use std::sync::Arc;

use betterlite_context::agentic_loop_plan::{FollowupMessage, JsonMap};
use betterlite_core::api::chat_agentic_loop::{ChatContinuation, ChatLoopResult};
use betterlite_core::operation::chat::ChatFailure;
use betterlite_core::responses_bridge::ResponsesBridgeCapabilities;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::execution::ChatError;
use betterlite_openai::stream::chunk::Chunk;
use futures_util::StreamExt as _;
use serde_json::value::RawValue;

use super::USER;
use super::support::{
    CheckingCallback, FakeFacts, FakeFoundry, Harness, RecordingTarget, args_from, chat_body, dig,
    json, request,
};
use crate::call_setup::correlation::CallOutcome;
use crate::call_wrapper::error::class_status;
use crate::call_wrapper::target::CallTarget;
use crate::completion_call::bridge_facts::ModelBridgeFacts;
use crate::completion_call::error::CompletionCallError;
use crate::completion_call::fallbacks::run_fallbacks;
use crate::completion_call::ports::NoFoundryRules;
use crate::completion_call::request;
use crate::completion_call::target_async::provider_of;
use crate::completion_request::args::JsonObject;
use crate::completion_request::call::{
    FallbackCall, ModelListCall, PreparedCall, PreparedCompletion,
};

// ---- target_ ----

#[test]
fn target_errors_carry_their_status() {
    let harness = Harness::new();
    let target = harness.target();
    let mapped = target.error(CompletionCallError::MissingMock, "gpt-4o");
    assert_eq!(class_status(&mapped.class), Some(400));
    let mapped = target.error(CompletionCallError::NoFallbackResponse, "gpt-4o");
    assert_eq!(class_status(&mapped.class), Some(500));
}

#[tokio::test]
async fn target_route_failure_keeps_its_status() {
    let harness = Harness::new();
    harness
        .chat
        .fails_with(ChatFailure::Route(ChatError::MissingApiKey));
    let target = harness.target();
    let call = request(&format!(r#"{{"model":"gpt-4o","messages":[{USER}]}}"#));
    let error = target.call(call, None).await.unwrap_err();
    assert_eq!(class_status(&error.class), Some(401));
}

#[test]
fn target_call_id_comes_from_the_kwarg_or_a_fresh_uuid() {
    let mut call = PreparedCall::default();
    uuid::Uuid::parse_str(&request::call_id(&call)).unwrap();
    call.kwargs.insert(
        String::from("litellm_call_id"),
        Json::Text(String::from("given-id")),
    );
    assert_eq!(request::call_id(&call), "given-id");
}

#[test]
fn target_names_the_completion_and_acompletion_entries() {
    let harness = Harness::new();
    assert_eq!(harness.target().name(), "completion");
    assert_eq!(harness.async_target().name(), "acompletion");
}

#[tokio::test]
async fn target_model_list_tries_the_next_deployment_after_an_empty_answer() {
    let harness = Harness::new();
    harness.chat.answers_null("empty-model");
    let target = harness.target();
    let deployments = vec![
        JsonObject::from([(
            String::from("model"),
            Json::Text(String::from("empty-model")),
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
    let CallOutcome::Response(answer) = outcome else {
        panic!("the second deployment answers");
    };
    assert_eq!(
        dig(&answer, &["id"]),
        Some(&Json::Text(String::from("chatcmpl-1")))
    );
}

#[tokio::test]
async fn target_model_list_keeps_litellms_deployment_parameter_leak() {
    let harness = Harness::new();
    harness.chat.answers_null("empty-model");
    let target = harness.target();
    let deployments = vec![
        JsonObject::from([
            (
                String::from("model"),
                Json::Text(String::from("empty-model")),
            ),
            (String::from("temperature"), json("0.4")),
        ]),
        JsonObject::from([(String::from("model"), Json::Text(String::from("gpt-4o")))]),
    ];
    let call = ModelListCall {
        deployments,
        args: args_from(&format!(r#"{{"model":"gpt-4o","messages":[{USER}]}}"#)),
    };
    target
        .run(PreparedCompletion::ModelList(call), false, None)
        .await
        .unwrap();
    let seen = harness.chat.seen();
    assert_eq!(seen.len(), 2);
    assert!(
        seen[1].body.contains("\"temperature\":0.4"),
        "{}",
        seen[1].body
    );
}

// ---- fallback_ ----

#[tokio::test]
async fn fallback_an_empty_answer_tries_the_next_entry() {
    let target = RecordingTarget::empty_then_ok();
    let args = args_from(&format!(r#"{{"model":"gpt-4o","messages":[{USER}]}}"#));
    let call = FallbackCall {
        fallbacks: vec![json(r#""gpt-3.5-turbo""#)],
        args,
    };
    let outcome = run_fallbacks(&target, call, None).await.unwrap();
    assert_eq!(target.call_map().len(), 2);
    let CallOutcome::Response(answer) = outcome else {
        panic!("the fallback answers");
    };
    assert_eq!(dig(&answer, &["id"]), Some(&Json::Text(String::from("ok"))));
    let attempt = dig(
        &answer,
        &[
            "_hidden_params",
            "additional_headers",
            "x-litellm-attempted-fallbacks",
        ],
    );
    assert!(matches!(attempt, Some(Json::Number(number)) if number.as_u64() == Some(1)));
}

#[tokio::test]
async fn fallback_keeps_the_original_request_parameters() {
    let target = RecordingTarget::failing_then_ok(1);
    let args = args_from(&format!(
        r#"{{"model":"gpt-4o","messages":[{USER}],"temperature":0.3}}"#
    ));
    let call = FallbackCall {
        fallbacks: vec![json(r#""gpt-3.5-turbo""#)],
        args,
    };
    run_fallbacks(&target, call, None).await.unwrap();
    let calls = target.call_map();
    assert_eq!(calls.len(), 2);
    assert!(calls[0].contains_key("messages"));
    assert!(matches!(
        calls[0].get("temperature"),
        Some(Json::Number(number)) if number.as_f64() == Some(0.3)
    ));
}

// ---- retry_ ----

#[tokio::test]
async fn retry_runs_the_completion_entry_and_the_async_entry() {
    let harness = Harness::new();
    let call = request(&format!(
        r#"{{"num_retries":1,"model":"gpt-4o","messages":[{USER}]}}"#
    ));
    let outcome = harness.target().retry(call, None).await.unwrap();
    let CallOutcome::Response(answer) = outcome else {
        panic!("the completion entry answers");
    };
    assert_eq!(
        dig(&answer, &["id"]),
        Some(&Json::Text(String::from("chatcmpl-1")))
    );
    let call = request(&format!(
        r#"{{"num_retries":1,"model":"gpt-4o","messages":[{USER}]}}"#
    ));
    let outcome = harness.async_target().retry(call, None).await.unwrap();
    let CallOutcome::Response(answer) = outcome else {
        panic!("the async entry answers");
    };
    assert_eq!(
        dig(&answer, &["id"]),
        Some(&Json::Text(String::from("chatcmpl-1")))
    );
}

// ---- async_ ----

#[test]
fn async_provider_of_reads_the_prepared_call() {
    let prepared = PreparedCompletion::Dispatch(PreparedCall {
        custom_llm_provider: Some(String::from("text-completion-openai")),
        ..PreparedCall::default()
    });
    assert_eq!(provider_of(&prepared), Some("text-completion-openai"));
}

#[tokio::test]
async fn async_continuation_answers_and_streams_through_the_call() {
    let harness = Harness::new();
    let target = harness.async_target();
    let messages = vec![FollowupMessage(
        RawValue::from_string(String::from(USER)).unwrap(),
    )];
    let mut params = JsonMap::new();
    params.insert(
        String::from("temperature"),
        RawValue::from_string(String::from("0.2")).unwrap(),
    );
    let kwargs = JsonMap::new();
    let result = ChatContinuation::complete(&target, "gpt-4o", &messages, &params, &kwargs)
        .await
        .unwrap();
    let ChatLoopResult::Response(raw) = result else {
        panic!("a buffered continuation answers a response");
    };
    let answer: Json = serde_json::from_str(raw.get()).unwrap();
    assert_eq!(
        dig(&answer, &["id"]),
        Some(&Json::Text(String::from("chatcmpl-1")))
    );
    harness.chat.streams(&[
        "data: {\"id\":\"chunk-1\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"he\"},\"finish_reason\":null}]}\n\n",
        "data: [DONE]\n\n",
    ]);
    let mut stream_params = JsonMap::new();
    stream_params.insert(
        String::from("stream"),
        RawValue::from_string(String::from("true")).unwrap(),
    );
    let result = ChatContinuation::complete(&target, "gpt-4o", &messages, &stream_params, &kwargs)
        .await
        .unwrap();
    let ChatLoopResult::Stream(stream) = result else {
        panic!("a streamed continuation answers a stream");
    };
    let chunks: Vec<Chunk> = stream.collect().await;
    assert_eq!(chunks.len(), 1);
    let encoded = serde_json::to_string(&chunks[0]).unwrap();
    assert!(encoded.contains("he"), "{encoded}");
}

#[tokio::test]
async fn async_agentic_loop_receives_the_call_details() {
    let mut harness = Harness::new();
    harness.callbacks.push(Arc::new(CheckingCallback {
        reply: chat_body("looped"),
    }));
    let target = harness.async_target();
    let call = request(&format!(
        r#"{{"model":"gpt-4o","messages":[{USER}],"temperature":0.2,"agentic_marker":"yes"}}"#
    ));
    let outcome = target.call(call, None).await.unwrap();
    let CallOutcome::Response(answer) = outcome else {
        panic!("the loop answers a response");
    };
    assert_eq!(
        dig(&answer, &["id"]),
        Some(&Json::Text(String::from("chatcmpl-agentic")))
    );
}

// ---- bridge_facts_ ----

#[test]
fn bridge_facts_no_foundry_rules_keep_azure_ai_on_chat() {
    let facts = FakeFacts::default();
    let configured = FakeFoundry::default();
    let bridge = ModelBridgeFacts {
        facts: &facts,
        foundry: &configured,
        openai: betterlite_config::sections::openai_credentials::Settings::default(),
        litellm_api_base: None,
    };
    assert!(!bridge.foundry_supports_responses("azure/gpt-5", None));
    assert!(!bridge.foundry_chat_rejects_tools("azure/gpt-5", None));
    let rules = NoFoundryRules;
    let bridge = ModelBridgeFacts {
        facts: &facts,
        foundry: &rules,
        openai: betterlite_config::sections::openai_credentials::Settings::default(),
        litellm_api_base: None,
    };
    assert!(!bridge.foundry_supports_responses("azure/gpt-5", None));
    assert!(!bridge.foundry_chat_rejects_tools("azure/gpt-5", None));
}
