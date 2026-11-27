#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_core::routes::CallType;
use betterlite_guardrails::guardrail_base::failure::{GuardrailFailure, ModifyResponseFailure};
use betterlite_guardrails::guardrail_message_scope::StreamItem;
use betterlite_guardrails::guardrail_translation::BaseTranslation as _;
use rstest::rstest;

use crate::guardrails::unified_guardrail::tests::support::{
    FakeGuardrail, FixedStamp, NoAssembler, NoHook, Registry, applied, blocked, no_call, object,
    registry, value,
};
use crate::guardrails::unified_guardrail::translation::{UnifiedTranslation, VariantFlags as _};

type Held = UnifiedTranslation<FakeGuardrail, FixedStamp, NoAssembler, NoHook>;

fn held(built: &Registry<FakeGuardrail>, call_type: CallType) -> &Held {
    &built.get(call_type).unwrap().translation
}

#[rstest]
#[case::completion(CallType::TextCompletion, false, false)]
#[case::embeddings(CallType::Embedding, false, false)]
#[case::image(CallType::ImageGeneration, false, false)]
#[case::speech(CallType::Speech, false, false)]
#[case::transcription(CallType::Transcription, false, false)]
#[case::video(CallType::VideoGeneration, false, false)]
#[case::chat(CallType::Completion, true, true)]
#[case::responses(CallType::Responses, true, true)]
#[case::anthropic(CallType::AnthropicMessages, true, true)]
#[case::endpoint(CallType::PassThroughEndpoint, false, false)]
#[case::llm_route(CallType::AllmPassthroughRoute, false, false)]
#[case::a2a(CallType::SendMessage, false, false)]
#[case::rerank(CallType::Rerank, false, false)]
#[case::google_genai(CallType::GenerateContent, false, false)]
#[case::mcp(CallType::CallMcpTool, false, false)]
fn class_flags_come_from_the_held_variant(
    #[case] call_type: CallType,
    #[case] delivers: bool,
    #[case] assembles: bool,
) {
    let built = registry::<FakeGuardrail>();
    let translation = held(&built, call_type);
    assert_eq!(translation.delivers_ended_stream_rewrites(), delivers);
    assert_eq!(translation.assembles_streamed_response(), assembles);
}

#[rstest]
#[case::completion(CallType::TextCompletion, r#"{"prompt": "secret"}"#)]
#[case::embeddings(CallType::Embedding, r#"{"input": "secret"}"#)]
#[case::image(CallType::ImageGeneration, r#"{"prompt": "secret"}"#)]
#[case::chat(
    CallType::Completion,
    r#"{"model": "m", "messages": [{"role": "user", "content": "secret"}]}"#
)]
#[case::responses(CallType::Responses, r#"{"model": "m", "input": "secret"}"#)]
#[case::anthropic(
    CallType::AnthropicMessages,
    r#"{"model": "m", "messages": [{"role": "user", "content": "secret"}]}"#
)]
#[case::rerank(CallType::Rerank, r#"{"query": "secret"}"#)]
#[case::google_genai(
    CallType::GenerateContent,
    r#"{"contents": [{"parts": [{"text": "secret"}]}]}"#
)]
#[tokio::test]
async fn input_rewrite_runs_the_held_variant(#[case] call_type: CallType, #[case] request: &str) {
    let built = registry::<FakeGuardrail>();
    let guardrail = FakeGuardrail::masking();
    let mut data = object(request);
    held(&built, call_type)
        .process_input_messages(&mut data, &applied(&guardrail), None)
        .await
        .unwrap();
    let sent = guardrail.seen_texts();
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(
        sent.iter().flatten().any(|text| text == "secret"),
        "{sent:?}"
    );
}

#[rstest]
#[case::completion(CallType::TextCompletion, r#"{"prompt": "secret"}"#)]
#[case::embeddings(CallType::Embedding, r#"{"input": "secret"}"#)]
#[case::chat(
    CallType::Completion,
    r#"{"model": "m", "messages": [{"role": "user", "content": "secret"}]}"#
)]
#[case::responses(CallType::Responses, r#"{"model": "m", "input": "secret"}"#)]
#[case::anthropic(
    CallType::AnthropicMessages,
    r#"{"model": "m", "messages": [{"role": "user", "content": "secret"}]}"#
)]
#[case::rerank(CallType::Rerank, r#"{"query": "secret"}"#)]
#[tokio::test]
async fn input_failure_comes_back_as_the_guardrail_failure(
    #[case] call_type: CallType,
    #[case] request: &str,
) {
    let built = registry::<FakeGuardrail>();
    let guardrail = FakeGuardrail::failing(blocked());
    let mut data = object(request);
    let failure = held(&built, call_type)
        .process_input_messages(&mut data, &applied(&guardrail), None)
        .await
        .unwrap_err();
    assert_eq!(failure, blocked());
}

#[tokio::test]
async fn output_rewrite_runs_the_held_variant() {
    let built = registry::<FakeGuardrail>();
    let guardrail = FakeGuardrail::masking();
    let response =
        value(r#"{"object": "text_completion", "choices": [{"index": 0, "text": "secret"}]}"#);
    let checked = held(&built, CallType::TextCompletion)
        .process_output_response(response, &applied(&guardrail), no_call())
        .await
        .unwrap();
    assert_eq!(guardrail.seen_texts(), vec![vec![String::from("secret")]]);
    assert_ne!(
        checked,
        value(r#"{"object": "text_completion", "choices": [{"index": 0, "text": "secret"}]}"#)
    );
}

#[tokio::test]
async fn streaming_scan_runs_the_held_variant() {
    let built = registry::<FakeGuardrail>();
    let guardrail = FakeGuardrail::failing(blocked());
    let mut frames = vec![StreamItem::Json(value(
        r#"{"candidates": [{"content": {"parts": [{"text": "secret"}]}}]}"#,
    ))];
    let failure = held(&built, CallType::GenerateContentStream)
        .process_streaming_response(&mut frames, &applied(&guardrail), no_call())
        .await
        .unwrap_err();
    assert_eq!(failure, blocked());
    held(&built, CallType::TextCompletion)
        .process_streaming_response(&mut frames, &applied(&guardrail), no_call())
        .await
        .unwrap();
}

#[test]
fn request_helpers_come_from_the_held_variant() {
    let built = registry::<FakeGuardrail>();
    let data = object(
        r#"{"messages": [{"role": "user", "content": "hi"}], "tools": [{"type": "function", "function": {"name": "lookup"}}]}"#,
    );
    let chat = held(&built, CallType::Completion);
    let completion = held(&built, CallType::TextCompletion);
    assert_eq!(
        chat.extract_request_tool_names(&data),
        vec![String::from("lookup")]
    );
    assert!(completion.extract_request_tool_names(&data).is_empty());
    assert!(chat.get_structured_messages(&data).is_some());
    assert!(completion.get_structured_messages(&data).is_none());
}

#[test]
fn stream_helpers_come_from_the_held_variant() {
    let built = registry::<FakeGuardrail>();
    let a2a = held(&built, CallType::SendMessage);
    let chat = held(&built, CallType::Completion);
    let completion = held(&built, CallType::TextCompletion);
    assert!(a2a.get_streaming_scan_key(&[]).is_some());
    assert!(completion.get_streaming_scan_key(&[]).is_none());
    let modify = GuardrailFailure::ModifyResponse(Box::new(ModifyResponseFailure {
        message: String::from("blocked"),
        model: String::from("m"),
        request_data: BTreeMap::new(),
        guardrail_name: None,
        detection_info: BTreeMap::new(),
        original_response: None,
    }));
    assert!(chat.build_block_sse_chunks(&modify, false, None).is_some());
    assert!(
        completion
            .build_block_sse_chunks(&modify, false, None)
            .is_none()
    );
    assert!(chat.build_stream_error_items(&blocked(), None).is_some());
    assert!(
        completion
            .build_stream_error_items(&blocked(), None)
            .is_none()
    );
}

#[test]
fn post_call_response_comes_from_the_held_variant() {
    let built = registry::<FakeGuardrail>();
    let response = value(
        r#"{"id": "chatcmpl-1", "object": "chat.completion", "created": 1, "model": "m", "choices": [{"index": 0, "message": {"role": "assistant", "content": "hi"}, "finish_reason": "stop"}], "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}}"#,
    );
    let completion = held(&built, CallType::TextCompletion);
    let anthropic = held(&built, CallType::AnthropicMessages);
    assert_eq!(
        completion.post_call_hook_response(response.clone()),
        response
    );
    assert_ne!(
        anthropic.post_call_hook_response(response.clone()),
        response
    );
}
