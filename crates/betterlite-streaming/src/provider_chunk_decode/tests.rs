#![cfg(test)]

use std::io::Error;
use std::path::Path;
use std::str::Utf8Error;
use std::{fs, str};

use rstest::rstest;
use serde_json::from_str;
use uuid::Uuid;

use crate::provider_chunk_decode::chunk::{
    DecodeState, DecodedChunk, ProviderChunk, ProviderKind, RequestOptions, as_array, as_object,
};
use crate::provider_chunk_decode::creator::chunk_creator;
use crate::provider_chunk_decode::dispatch::decode_provider_chunk;
use crate::provider_chunk_decode::error::ProviderChunkError;
use crate::provider_chunk_decode::fuzz_input::decode_fuzzed_stream_chunk;
use crate::stream_chunk_shaping::ChunkShaper;
use crate::stream_chunk_shaping::creation::StreamInstant;
use crate::stream_chunk_shaping::records::{
    GenericChunk, JsonObject, JsonValue, StreamChoice, StreamChunk, Usage,
};

fn options() -> RequestOptions {
    RequestOptions {
        send_stream_usage: false,
        is_function_call: false,
    }
}

fn state(model: &str) -> DecodeState {
    DecodeState {
        model: model.to_owned(),
        kind: ProviderKind::Builtin,
        received_finish_reason: None,
        intermittent_finish_reason: None,
        sent_first_chunk: false,
        complete_response: String::new(),
        completion_stream: None,
        options: options(),
        repeated_chunk_limit: 100,
    }
}

fn custom_state(model: &str) -> DecodeState {
    let mut decode_state = state(model);
    decode_state.kind = ProviderKind::Custom;
    decode_state
}

fn json(source: &str) -> JsonValue {
    from_str(source).expect("the test literal is valid JSON")
}

fn json_chunk(source: &str) -> ProviderChunk {
    ProviderChunk::Json(Box::new(json(source)))
}

fn generic_chunk(source: &str) -> ProviderChunk {
    let parsed: GenericChunk = from_str(source).expect("the test literal is a generic chunk");
    ProviderChunk::Generic(Box::new(parsed))
}

fn text(value: &str) -> ProviderChunk {
    ProviderChunk::Text(value.to_owned())
}

fn decode(provider: &str, chunk: &ProviderChunk, decode_state: &mut DecodeState) -> DecodedChunk {
    decode_provider_chunk(Some(provider), chunk, decode_state).expect("the chunk decodes")
}

fn decode_error(
    provider: &str,
    chunk: &ProviderChunk,
    decode_state: &mut DecodeState,
) -> ProviderChunkError {
    decode_provider_chunk(Some(provider), chunk, decode_state).expect_err("the chunk fails")
}

fn fixed_instant() -> StreamInstant {
    StreamInstant {
        seconds: 1_700_000_000,
        timestamp: 1_700_000_000.5,
    }
}

fn fixed_id() -> Uuid {
    Uuid::nil()
}

fn shaper(provider: &str) -> ChunkShaper {
    let mut shaping = ChunkShaper::new(
        "gpt-4".to_owned(),
        Some(provider.to_owned()),
        false,
        JsonObject::new(),
    );
    shaping.now = fixed_instant;
    shaping.next_id = fixed_id;
    shaping
}

fn created(
    provider: &str,
    decode_state: &mut DecodeState,
    chunk: &ProviderChunk,
) -> Option<StreamChunk> {
    let mut shaping = shaper(provider);
    chunk_creator(&mut shaping, decode_state, chunk).expect("the chunk is shaped")
}

fn usage(prompt: i64, completion: i64, total: i64) -> Usage {
    Usage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: total,
        cost: None,
        prompt_tokens_details: None,
        completion_tokens_details: None,
        extra: JsonObject::new(),
    }
}

fn first_choice(chunk: &StreamChunk) -> &StreamChoice {
    chunk.choices.first().expect("the chunk has a choice")
}

#[test]
fn generic_chunk_carries_text_and_fields() {
    let mut decode_state = state("gpt-4");
    let chunk = generic_chunk(
        r#"{"text": "hi", "tool_use": null, "is_finished": false, "finish_reason": "", "usage": null, "provider_specific_fields": null}"#,
    );
    let decoded = decode("openai", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "hi");
    assert!(!decoded.is_finished);
    assert_eq!(decoded.finish_reason.as_deref(), Some(""));
    assert!(decoded.usage.is_none());
}

#[test]
fn generic_json_chunk_takes_the_generic_branch_for_a_builtin_provider() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"text": "hello", "is_finished": true, "finish_reason": "stop", "usage": null}"#,
    );
    let decoded = decode("openai", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "hello");
    assert!(decoded.is_finished);
    assert_eq!(decode_state.received_finish_reason.as_deref(), Some("stop"));
    assert_eq!(
        decode_state.intermittent_finish_reason.as_deref(),
        Some("stop")
    );
}

#[test]
fn generic_tool_use_becomes_a_one_element_tool_calls_list() {
    let mut decode_state = state("gpt-4");
    let chunk = generic_chunk(
        r#"{"text": "", "tool_use": {"id": "call_1", "type": "function", "function": {"name": "f", "arguments": "{}"}}, "is_finished": false, "finish_reason": "", "usage": null}"#,
    );
    let decoded = decode("openai", &chunk, &mut decode_state);
    let calls = decoded.tool_calls.as_ref().expect("the chunk has calls");
    let items = as_array(calls).expect("tool calls are a list");
    assert_eq!(items.len(), 1);
    assert_eq!(decoded.tool_use, items.first().cloned());
}

#[test]
fn generic_usage_and_provider_fields_are_carried() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"text": "x", "is_finished": false, "finish_reason": "", "usage": {"prompt_tokens": 3, "completion_tokens": 4, "total_tokens": 7}, "provider_specific_fields": {"trace": "t1"}}"#,
    );
    let decoded = decode("openai", &chunk, &mut decode_state);
    assert_eq!(decoded.usage, Some(usage(3, 4, 7)));
    let fields = decoded
        .provider_specific_fields
        .expect("provider fields are carried");
    assert_eq!(fields.get("trace").and_then(JsonValue::as_str), Some("t1"));
}

#[test]
fn generic_stop_iteration_after_a_finish_reason_without_content() {
    let mut decode_state = state("gpt-4");
    decode_state.received_finish_reason = Some("stop".to_owned());
    let chunk =
        generic_chunk(r#"{"text": "", "is_finished": false, "finish_reason": "", "usage": null}"#);
    let error = decode_error("openai", &chunk, &mut decode_state);
    assert!(matches!(error, ProviderChunkError::StopIteration));
}

#[test]
fn generic_provider_fields_keep_an_empty_chunk_after_a_finish_reason() {
    let mut decode_state = state("gpt-4");
    decode_state.received_finish_reason = Some("stop".to_owned());
    let chunk = json_chunk(
        r#"{"text": "", "is_finished": false, "finish_reason": "", "usage": null, "provider_specific_fields": null}"#,
    );
    let decoded = decode("openai", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "");
}

#[test]
fn generic_content_after_a_finish_reason_is_accepted() {
    let mut decode_state = state("gpt-4");
    decode_state.received_finish_reason = Some("stop".to_owned());
    let chunk = generic_chunk(
        r#"{"text": "more", "is_finished": false, "finish_reason": "", "usage": null}"#,
    );
    let decoded = decode("openai", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "more");
}

#[test]
fn generic_custom_provider_accepts_a_generic_chunk() {
    let mut decode_state = custom_state("my-model");
    let chunk = generic_chunk(
        r#"{"text": "custom", "is_finished": true, "finish_reason": "stop", "usage": null}"#,
    );
    let decoded = decode("my-provider", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "custom");
    assert!(decoded.is_finished);
}

#[test]
fn text_providers_predibase_reads_the_token_text() {
    let mut decode_state = state("gpt-4");
    let chunk = text(r#"data:{"token": {"text": "hi"}}"#);
    let decoded = decode("predibase", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "hi");
    assert!(!decoded.is_finished);
}

#[test]
fn text_providers_predibase_takes_the_details_finish_reason() {
    let mut decode_state = state("gpt-4");
    let chunk = text(r#"data:{"details": {"finish_reason": "length"}}"#);
    let decoded = decode("predibase", &chunk, &mut decode_state);
    assert!(decoded.is_finished);
    assert_eq!(decoded.finish_reason.as_deref(), Some("length"));
    assert_eq!(
        decode_state.received_finish_reason.as_deref(),
        Some("length")
    );
}

#[test]
fn text_providers_predibase_generated_text_ends_the_stream() {
    let mut decode_state = state("gpt-4");
    let chunk = text(r#"data:{"generated_text": "full answer"}"#);
    let decoded = decode("predibase", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "");
    assert!(decoded.is_finished);
    assert_eq!(decoded.finish_reason.as_deref(), Some("stop"));
}

#[test]
fn text_providers_predibase_reports_an_error_field() {
    let mut decode_state = state("gpt-4");
    let chunk = text(r#"data:{"error": "boom"}"#);
    let error = decode_error("predibase", &chunk, &mut decode_state);
    assert_eq!(error.to_string(), "boom");
}

#[test]
fn text_providers_predibase_rejects_a_plain_error_line() {
    let mut decode_state = state("gpt-4");
    let error = decode_error("predibase", &text("an error happened"), &mut decode_state);
    assert!(matches!(error, ProviderChunkError::RejectedResponse { .. }));
}

#[test]
fn text_providers_predibase_plain_line_is_empty() {
    let mut decode_state = state("gpt-4");
    let decoded = decode("predibase", &text("plain"), &mut decode_state);
    assert_eq!(decoded.text, "");
    assert!(!decoded.is_finished);
}

#[test]
fn text_providers_predibase_accepts_bytes() {
    let mut decode_state = state("gpt-4");
    let chunk = ProviderChunk::Bytes(br#"data:{"token": {"text": "hi"}}"#.to_vec());
    let decoded = decode("predibase", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "hi");
}

#[test]
fn text_providers_nlp_cloud_reads_generated_text() {
    let mut decode_state = state("gpt-4");
    let chunk = text(r#"{"generated_text": "hello"}"#);
    let decoded = decode("nlp_cloud", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "hello");
}

#[test]
fn text_providers_nlp_cloud_done_ends_the_stream() {
    let mut decode_state = state("gpt-4");
    let chunk = text(r#"{"generated_text": "hello [DONE]"}"#);
    let decoded = decode("nlp_cloud", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "hello ");
    assert!(decoded.is_finished);
    assert_eq!(decoded.finish_reason.as_deref(), Some("stop"));
}

#[test]
fn text_providers_nlp_cloud_dolphin_strips_the_complete_response() {
    let mut decode_state = state("dolphin-2.2");
    let first = decode("nlp_cloud", &text(r#""Hello"#), &mut decode_state);
    assert_eq!(first.text, "\"Hello");
    assert_eq!(decode_state.complete_response, "\"Hello");
    let second = decode("nlp_cloud", &text(r#""Hello world""#), &mut decode_state);
    assert_eq!(second.text, " world\"");
    assert_eq!(decode_state.complete_response, "\"Hello world\"");
}

#[test]
fn text_providers_nlp_cloud_after_first_chunk_recovers_with_stop() {
    let mut decode_state = state("gpt-4");
    decode_state.sent_first_chunk = true;
    let decoded = decode("nlp_cloud", &text("not json"), &mut decode_state);
    assert_eq!(decoded.text, "");
    assert_eq!(decode_state.received_finish_reason.as_deref(), Some("stop"));
}

#[test]
fn text_providers_aleph_alpha_takes_the_first_completion() {
    let mut decode_state = state("gpt-4");
    let chunk = ProviderChunk::Bytes(br#"{"completions": [{"completion": "hi"}]}"#.to_vec());
    let decoded = decode("aleph_alpha", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "hi");
    assert!(decoded.is_finished);
    assert_eq!(decoded.finish_reason.as_deref(), Some("stop"));
}

#[test]
fn text_providers_aleph_alpha_missing_completion_is_rejected() {
    let mut decode_state = state("gpt-4");
    let error = decode_error(
        "aleph_alpha",
        &text(r#"{"completions": []}"#),
        &mut decode_state,
    );
    assert!(matches!(error, ProviderChunkError::RejectedResponse { .. }));
}

#[test]
fn text_providers_replicate_reads_output() {
    let mut decode_state = state("gpt-4");
    let decoded = decode(
        "replicate",
        &json_chunk(r#"{"output": "hi"}"#),
        &mut decode_state,
    );
    assert_eq!(decoded.text, "hi");
    assert!(!decoded.is_finished);
}

#[test]
fn text_providers_replicate_succeeded_status_finishes() {
    let mut decode_state = state("gpt-4");
    let decoded = decode(
        "replicate",
        &json_chunk(r#"{"status": "succeeded"}"#),
        &mut decode_state,
    );
    assert!(decoded.is_finished);
    assert_eq!(decoded.finish_reason.as_deref(), Some("stop"));
    assert_eq!(decode_state.received_finish_reason.as_deref(), Some("stop"));
}

#[test]
fn text_providers_replicate_model_name_selects_the_reader() {
    let mut decode_state = state("replicate");
    let decoded = decode(
        "openai",
        &json_chunk(r#"{"status": "processing", "output": "hi"}"#),
        &mut decode_state,
    );
    assert_eq!(decoded.text, "hi");
}

#[test]
fn text_providers_replicate_error_is_rejected() {
    let mut decode_state = state("gpt-4");
    let error = decode_error(
        "replicate",
        &json_chunk(r#"{"error": "bad"}"#),
        &mut decode_state,
    );
    assert!(matches!(error, ProviderChunkError::RejectedResponse { .. }));
}

#[test]
fn text_providers_replicate_rejects_a_non_text_output() {
    let mut decode_state = state("gpt-4");
    let error = decode_error(
        "replicate",
        &json_chunk(r#"{"output": [1, 2]}"#),
        &mut decode_state,
    );
    assert!(matches!(
        error,
        ProviderChunkError::InvalidField { field: "output" }
    ));
}

#[test]
fn text_providers_vllm_takes_the_first_output_text() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(r#"[{"outputs": [{"text": "hi"}]}]"#);
    let decoded = decode("vllm", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "hi");
}

#[test]
fn text_providers_vllm_missing_output_is_an_error() {
    let mut decode_state = state("gpt-4");
    let error = decode_error(
        "vllm",
        &json_chunk(r#"[{"outputs": []}]"#),
        &mut decode_state,
    );
    assert!(matches!(error, ProviderChunkError::MissingField { .. }));
}

#[test]
fn text_providers_vllm_rejects_a_non_list_chunk() {
    let mut decode_state = state("gpt-4");
    let error = decode_error("vllm", &json_chunk(r#"{"outputs": []}"#), &mut decode_state);
    assert!(matches!(error, ProviderChunkError::UnexpectedChunk { .. }));
}

#[test]
fn text_providers_petals_slices_thirty_characters() {
    let mut decode_state = state("gpt-4");
    decode_state.completion_stream = Some("a".repeat(40));
    let decoded = decode("petals", &text("ignored"), &mut decode_state);
    assert_eq!(decoded.text, "a".repeat(30));
    assert_eq!(
        decode_state.completion_stream.as_deref(),
        Some("a".repeat(10).as_str())
    );
}

#[test]
fn text_providers_petals_slices_by_characters_not_bytes() {
    let mut decode_state = state("gpt-4");
    decode_state.completion_stream = Some("\u{e9}".repeat(31));
    let decoded = decode("petals", &text("ignored"), &mut decode_state);
    assert_eq!(decoded.text, "\u{e9}".repeat(30));
    assert_eq!(decode_state.completion_stream.as_deref(), Some("\u{e9}"));
}

#[test]
fn text_providers_petals_empty_stream_sets_stop() {
    let mut decode_state = state("gpt-4");
    let decoded = decode("petals", &text("ignored"), &mut decode_state);
    assert_eq!(decoded.text, "");
    assert_eq!(decode_state.received_finish_reason.as_deref(), Some("stop"));
}

#[test]
fn text_providers_petals_stops_once_finished() {
    let mut decode_state = state("gpt-4");
    decode_state.received_finish_reason = Some("stop".to_owned());
    let error = decode_error("petals", &text("ignored"), &mut decode_state);
    assert!(matches!(error, ProviderChunkError::StopIteration));
}

#[test]
fn text_providers_triton_reads_the_object() {
    let mut decode_state = state("gpt-4");
    let decoded = decode(
        "triton",
        &json_chunk(r#"{"text_output": "hi", "stop_reason": "stop", "is_finished": true}"#),
        &mut decode_state,
    );
    assert_eq!(decoded.text, "hi");
    assert!(decoded.is_finished);
    assert_eq!(decode_state.received_finish_reason.as_deref(), Some("stop"));
}

#[test]
fn text_providers_triton_reads_an_sse_text_line() {
    let mut decode_state = state("gpt-4");
    let chunk = text(r#"data: {"text_output": "hi", "is_finished": false}"#);
    let decoded = decode("triton", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "hi");
}

#[test]
fn text_providers_triton_line_without_text_output_is_empty() {
    let mut decode_state = state("gpt-4");
    let decoded = decode("triton", &text("nothing here"), &mut decode_state);
    assert_eq!(decoded.text, "");
    assert!(!decoded.is_finished);
}

#[test]
fn text_providers_text_completion_openai_keeps_text_and_usage() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"choices": [{"text": "hi", "finish_reason": "stop"}], "usage": {"prompt_tokens": 2, "completion_tokens": 3, "total_tokens": 5, "cost": 1}}"#,
    );
    let decoded = decode("text-completion-openai", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "hi");
    assert!(decoded.is_finished);
    assert_eq!(decoded.finish_reason.as_deref(), Some("stop"));
    assert_eq!(decoded.usage, Some(usage(2, 3, 5)));
}

#[test]
fn text_providers_azure_text_reads_choices() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(r#"{"choices": [{"text": "hi", "finish_reason": "length"}]}"#);
    let decoded = decode("azure_text", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "hi");
    assert_eq!(decoded.finish_reason.as_deref(), Some("length"));
    assert!(decoded.usage.is_none());
}

#[test]
fn text_providers_azure_text_ignores_a_non_object_chunk() {
    let mut decode_state = state("gpt-4");
    let decoded = decode("azure_text", &text("ignored"), &mut decode_state);
    assert_eq!(decoded.text, "");
}

#[test]
fn text_providers_codestral_parses_the_delta_content() {
    let mut decode_state = state("gpt-4");
    let chunk = text(
        r#"data: {"id": "c", "choices": [{"index": 0, "delta": {"content": "hi"}, "finish_reason": null}]}"#,
    );
    let decoded = decode("text-completion-codestral", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "hi");
    assert!(!decoded.is_finished);
    assert!(decoded.original.is_some());
}

#[test]
fn text_providers_codestral_done_is_empty() {
    let mut decode_state = state("gpt-4");
    let decoded = decode(
        "text-completion-codestral",
        &text("data: [DONE]"),
        &mut decode_state,
    );
    assert_eq!(decoded.text, "");
    assert!(decoded.original.is_none());
}

#[test]
fn text_providers_codestral_invalid_json_is_empty() {
    let mut decode_state = state("gpt-4");
    let decoded = decode(
        "text-completion-codestral",
        &text("data: {not json"),
        &mut decode_state,
    );
    assert_eq!(decoded.text, "");
}

#[test]
fn text_providers_codestral_finish_reason_finishes() {
    let mut decode_state = state("gpt-4");
    let chunk = text(
        r#"{"choices": [{"delta": {"content": ""}, "finish_reason": "stop", "logprobs": {"a": 1}}]}"#,
    );
    let decoded = decode("text-completion-codestral", &chunk, &mut decode_state);
    assert!(decoded.is_finished);
    assert_eq!(decoded.finish_reason.as_deref(), Some("stop"));
    assert!(decoded.logprobs.is_some());
    assert_eq!(decode_state.received_finish_reason.as_deref(), Some("stop"));
}

#[test]
fn openai_keeps_usage_from_the_chunk() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"id": "c1", "choices": [{"index": 0, "delta": {"content": "hi"}}], "usage": {"prompt_tokens": 2, "completion_tokens": 3, "total_tokens": 5}}"#,
    );
    let decoded = decode("openai", &chunk, &mut decode_state);
    assert_eq!(decoded.usage, Some(usage(2, 3, 5)));
    assert_eq!(decoded.text, "hi");
}

#[test]
fn openai_takes_the_model_from_an_azure_chunk() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"id": "c1", "model": "azure-deployment", "choices": [{"index": 0, "delta": {"content": "hi"}}]}"#,
    );
    let decoded = decode("azure", &chunk, &mut decode_state);
    assert_eq!(decode_state.model, "azure-deployment");
    assert_eq!(decoded.text, "hi");
}

#[test]
fn openai_azure_ai_reads_the_model_too() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"model": "ai-deployment", "choices": [{"index": 0, "delta": {"content": "hi"}}]}"#,
    );
    decode("azure_ai", &chunk, &mut decode_state);
    assert_eq!(decode_state.model, "ai-deployment");
}

#[test]
fn openai_plain_provider_keeps_the_model() {
    let mut decode_state = state("gpt-4");
    let chunk =
        json_chunk(r#"{"model": "other", "choices": [{"index": 0, "delta": {"content": "hi"}}]}"#);
    decode("openai", &chunk, &mut decode_state);
    assert_eq!(decode_state.model, "gpt-4");
}

#[test]
fn openai_finish_reason_sets_the_received_reason() {
    let mut decode_state = state("gpt-4");
    let chunk =
        json_chunk(r#"{"choices": [{"delta": {"content": "hi"}, "finish_reason": "stop"}]}"#);
    let decoded = decode("openai", &chunk, &mut decode_state);
    assert!(decoded.is_finished);
    assert_eq!(decoded.finish_reason.as_deref(), Some("stop"));
    assert_eq!(decode_state.received_finish_reason.as_deref(), Some("stop"));
    assert_eq!(
        decode_state.intermittent_finish_reason.as_deref(),
        Some("stop")
    );
}

#[test]
fn openai_empty_finish_reason_is_no_finish() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(r#"{"choices": [{"delta": {"content": "hi"}, "finish_reason": ""}]}"#);
    let decoded = decode("openai", &chunk, &mut decode_state);
    assert!(!decoded.is_finished);
    assert!(decoded.finish_reason.is_none());
    assert!(decode_state.intermittent_finish_reason.is_none());
}

#[test]
fn openai_clears_the_intermittent_reason_on_a_chunk_without_one() {
    let mut decode_state = state("gpt-4");
    decode_state.intermittent_finish_reason = Some("length".to_owned());
    let chunk = json_chunk(r#"{"choices": [{"delta": {"content": "hi"}}]}"#);
    decode("openai", &chunk, &mut decode_state);
    assert!(decode_state.intermittent_finish_reason.is_none());
}

#[test]
fn openai_hidden_params_is_finished_overrides_the_flag() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"choices": [{"delta": {"content": "hi"}, "finish_reason": "stop"}], "hidden_params": {"is_finished": false}}"#,
    );
    let decoded = decode("openai", &chunk, &mut decode_state);
    assert!(!decoded.is_finished);
    assert!(decode_state.received_finish_reason.is_none());
}

#[test]
fn openai_logprobs_are_carried() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"choices": [{"delta": {"content": "hi"}, "logprobs": {"tokens": ["hi"]}}]}"#,
    );
    let decoded = decode("openai", &chunk, &mut decode_state);
    assert!(decoded.logprobs.is_some());
}

#[test]
fn openai_system_fingerprint_is_recorded() {
    let mut decode_state = state("gpt-4");
    let chunk =
        json_chunk(r#"{"system_fingerprint": "fp-1", "choices": [{"delta": {"content": "hi"}}]}"#);
    let decoded = decode("openai", &chunk, &mut decode_state);
    assert_eq!(decoded.system_fingerprint, Some(Some("fp-1".to_owned())));
    assert_eq!(
        decoded.response_system_fingerprint,
        Some(Some("fp-1".to_owned()))
    );
}

#[test]
fn openai_missing_choices_gives_empty_text() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(r#"{"id": "c1"}"#);
    let decoded = decode("openai", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "");
    assert!(!decoded.is_finished);
    assert!(decoded.original.is_some());
}

#[test]
fn openai_error_finish_reason_fails() {
    let mut decode_state = state("gpt-4");
    let chunk =
        json_chunk(r#"{"choices": [{"delta": {"content": "hi"}, "finish_reason": "error"}]}"#);
    let error = decode_error("openai", &chunk, &mut decode_state);
    assert!(matches!(error, ProviderChunkError::StreamingFailure { .. }));
}

#[test]
fn errors_nlp_cloud_message_matches_litellm() {
    let mut decode_state = state("gpt-4");
    let error = decode_error("nlp_cloud", &text("not json"), &mut decode_state);
    assert_eq!(
        error.to_string(),
        "Unable to parse response. Original response: not json"
    );
}

#[test]
fn errors_nlp_cloud_unknown_stream_before_the_first_chunk() {
    let mut decode_state = state("gpt-4");
    let error = decode_error("nlp_cloud", &text("not json"), &mut decode_state);
    assert!(matches!(error, ProviderChunkError::UnknownStream));
}

#[test]
fn errors_nlp_cloud_keeps_the_error_after_a_finish_reason() {
    let mut decode_state = state("gpt-4");
    decode_state.received_finish_reason = Some("stop".to_owned());
    let error = decode_error("nlp_cloud", &text("not json"), &mut decode_state);
    assert!(matches!(error, ProviderChunkError::ParseResponse { .. }));
}

#[test]
fn errors_vertex_blocked_message() {
    let mut decode_state = state("gpt-4");
    let chunk =
        json_chunk(r#"{"candidates": [{"content": {"parts": []}, "finish_reason": "SAFETY"}]}"#);
    let error = decode_error("vertex_ai", &chunk, &mut decode_state);
    assert_eq!(error.to_string(), "The response was blocked by VertexAI.");
}

#[test]
fn errors_vertex_maps_the_finish_reason() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"candidates": [{"content": {"parts": [{"text": "hi"}]}, "finish_reason": "STOP"}]}"#,
    );
    let decoded = decode("vertex_ai", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "hi");
    assert_eq!(decode_state.received_finish_reason.as_deref(), Some("stop"));
}

#[test]
fn errors_vertex_unspecified_finish_reason_is_ignored() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"candidates": [{"content": {"parts": [{"text": "hi"}]}, "finish_reason": "FINISH_REASON_UNSPECIFIED"}]}"#,
    );
    decode("vertex_ai", &chunk, &mut decode_state);
    assert!(decode_state.received_finish_reason.is_none());
}

#[test]
fn errors_predibase_error_message() {
    let mut decode_state = state("gpt-4");
    let error = decode_error(
        "predibase",
        &text(r#"data:{"error": "boom"}"#),
        &mut decode_state,
    );
    assert_eq!(error.to_string(), "boom");
}

#[test]
fn errors_replicate_error_message_contains_the_chunk() {
    let mut decode_state = state("gpt-4");
    let error = decode_error(
        "replicate",
        &json_chunk(r#"{"error": "bad"}"#),
        &mut decode_state,
    );
    assert_eq!(
        error.to_string(),
        r#"Unable to parse response. Original response: {"error":"bad"}"#
    );
}

#[test]
fn errors_codestral_rejects_a_non_text_chunk() {
    let mut decode_state = state("gpt-4");
    let error = decode_error(
        "text-completion-codestral",
        &json_chunk(r#"{"choices": []}"#),
        &mut decode_state,
    );
    assert!(matches!(error, ProviderChunkError::NotAString { .. }));
}

#[test]
fn errors_invalid_utf8_bytes_are_refused() {
    let mut decode_state = state("gpt-4");
    let chunk = ProviderChunk::Bytes(vec![0xff, 0xfe]);
    let error = decode_error("predibase", &chunk, &mut decode_state);
    assert!(matches!(error, ProviderChunkError::InvalidUtf8 { .. }));
}

#[rstest]
#[case::stop_iteration(ProviderChunkError::StopIteration, "StopIteration")]
#[case::unknown_stream(ProviderChunkError::UnknownStream, "Exception")]
#[case::vertex_blocked(ProviderChunkError::VertexBlocked, "Exception")]
#[case::not_a_string(
    ProviderChunkError::NotAString { chunk: String::new() },
    "ValueError"
)]
#[case::missing_field(
    ProviderChunkError::MissingField { field: "text" },
    "TypeError"
)]
#[case::invalid_utf8(
    ProviderChunkError::InvalidUtf8 {
        source: invalid_utf8()
    },
    "UnicodeDecodeError"
)]
fn errors_class_names(#[case] error: ProviderChunkError, #[case] class: &str) {
    assert_eq!(error.class(), class);
}

fn invalid_utf8() -> Utf8Error {
    let bytes = vec![0xff_u8];
    str::from_utf8(&bytes).expect_err("the bytes are invalid")
}

#[test]
fn creator_fills_the_role_of_a_plain_chunk() {
    let mut decode_state = state("gpt-4");
    let chunk =
        json_chunk(r#"{"id": "c1", "choices": [{"index": 0, "delta": {"content": "hi"}}]}"#);
    let created_chunk = created("openai", &mut decode_state, &chunk).expect("a chunk is emitted");
    let choice = first_choice(&created_chunk);
    assert_eq!(choice.delta.content.as_deref(), Some("hi"));
    assert_eq!(choice.delta.role.as_deref(), Some("assistant"));
    assert_eq!(created_chunk.model, "gpt-4");
    assert_eq!(created_chunk.id, "c1");
}

#[test]
fn creator_drops_the_role_after_the_first_chunk() {
    let mut decode_state = state("gpt-4");
    let first = json_chunk(r#"{"id": "c1", "choices": [{"delta": {"content": "a"}}]}"#);
    created("openai", &mut decode_state, &first).expect("the first chunk is emitted");
    let second = json_chunk(r#"{"id": "c1", "choices": [{"delta": {"content": "b"}}]}"#);
    let created_chunk =
        created("openai", &mut decode_state, &second).expect("the second is emitted");
    assert_eq!(first_choice(&created_chunk).delta.role, None);
    assert!(decode_state.sent_first_chunk);
}

#[test]
fn creator_early_return_passes_the_custom_chunk_through() {
    let mut decode_state = custom_state("my-model");
    let chunk = json_chunk(
        r#"{"id": "c1", "choices": [{"index": 0, "delta": {"content": "hi"}, "finish_reason": "stop"}]}"#,
    );
    let created_chunk =
        created("my-provider", &mut decode_state, &chunk).expect("the chunk passes");
    let choice = first_choice(&created_chunk);
    assert_eq!(choice.delta.content.as_deref(), Some("hi"));
    assert_eq!(choice.finish_reason, None);
    assert_eq!(decode_state.received_finish_reason.as_deref(), Some("stop"));
}

#[test]
fn creator_custom_finish_only_chunk_is_skipped() {
    let mut decode_state = custom_state("my-model");
    let chunk = json_chunk(
        r#"{"id": "c1", "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}"#,
    );
    let emitted = created("my-provider", &mut decode_state, &chunk);
    assert!(emitted.is_none());
    assert_eq!(decode_state.received_finish_reason.as_deref(), Some("stop"));
}

#[test]
fn creator_tool_use_becomes_function_call_when_functions_were_sent() {
    let mut decode_state = state("gpt-4");
    decode_state.options.is_function_call = true;
    let chunk = generic_chunk(
        r#"{"text": "", "tool_use": {"id": "call_1", "type": "function", "function": {"name": "f", "arguments": "{}"}}, "is_finished": false, "finish_reason": "", "usage": null}"#,
    );
    let created_chunk = created("openai", &mut decode_state, &chunk).expect("a chunk is emitted");
    let delta = &first_choice(&created_chunk).delta;
    assert!(delta.tool_calls.is_none());
    let function = delta
        .function_call
        .as_ref()
        .expect("the call became a function call");
    assert_eq!(
        as_object(function)
            .and_then(|fields| fields.get("name"))
            .and_then(JsonValue::as_str),
        Some("f")
    );
}

#[test]
fn creator_tool_use_stays_tool_calls_without_functions() {
    let mut decode_state = state("gpt-4");
    let chunk = generic_chunk(
        r#"{"text": "", "tool_use": {"id": "call_1", "type": "function", "function": {"name": "f", "arguments": "{}"}}, "is_finished": false, "finish_reason": "", "usage": null}"#,
    );
    let created_chunk = created("openai", &mut decode_state, &chunk).expect("a chunk is emitted");
    let delta = &first_choice(&created_chunk).delta;
    assert!(delta.function_call.is_none());
    assert!(delta.tool_calls.is_some());
}

#[test]
fn creator_usage_only_chunk_is_recorded_without_send_usage() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"id": "c1", "choices": [], "usage": {"prompt_tokens": 2, "completion_tokens": 3, "total_tokens": 5}}"#,
    );
    let mut shaping = shaper("openai");
    let emitted = chunk_creator(&mut shaping, &mut decode_state, &chunk).expect("the chunk shapes");
    assert!(emitted.is_none());
    assert_eq!(shaping.chunks.len(), 1);
    assert!(
        shaping
            .chunks
            .first()
            .expect("the recorded chunk")
            .choices
            .is_empty()
    );
}

#[test]
fn creator_usage_only_chunk_is_emitted_with_send_usage() {
    let mut decode_state = state("gpt-4");
    decode_state.options.send_stream_usage = true;
    let chunk = json_chunk(
        r#"{"id": "c1", "choices": [], "usage": {"prompt_tokens": 2, "completion_tokens": 3, "total_tokens": 5}}"#,
    );
    let created_chunk = created("openai", &mut decode_state, &chunk).expect("the chunk is emitted");
    assert!(created_chunk.choices.is_empty());
    assert_eq!(created_chunk.usage, Some(usage(2, 3, 5)));
}

#[test]
fn creator_azure_model_reaches_the_chunk() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"id": "c1", "model": "azure-deployment", "choices": [{"delta": {"content": "hi"}}]}"#,
    );
    let created_chunk = created("azure", &mut decode_state, &chunk).expect("a chunk is emitted");
    assert_eq!(created_chunk.model, "azure-deployment");
}

#[test]
fn creator_generic_provider_fields_land_on_the_chunk() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"text": "hi", "is_finished": false, "finish_reason": "", "usage": null, "provider_specific_fields": {"provider_specific_fields": {"x": 1}}}"#,
    );
    let created_chunk = created("openai", &mut decode_state, &chunk).expect("a chunk is emitted");
    let fields = created_chunk
        .provider_specific_fields
        .expect("the provider fields are set");
    assert!(fields.contains_key("x"));
}

#[test]
fn creator_system_fingerprint_comes_from_the_chunk() {
    let mut decode_state = state("gpt-4");
    let chunk =
        json_chunk(r#"{"system_fingerprint": "fp-2", "choices": [{"delta": {"content": "hi"}}]}"#);
    let mut shaping = shaper("openai");
    let created_chunk = chunk_creator(&mut shaping, &mut decode_state, &chunk)
        .expect("the chunk shapes")
        .expect("a chunk is emitted");
    assert_eq!(created_chunk.system_fingerprint.as_deref(), Some("fp-2"));
    assert_eq!(shaping.system_fingerprint.as_deref(), Some("fp-2"));
}

#[test]
fn creator_finish_only_chunk_is_the_terminal_chunk() {
    let mut decode_state = state("gpt-4");
    let chunk =
        json_chunk(r#"{"id": "c1", "choices": [{"delta": {}, "finish_reason": "length"}]}"#);
    let mut shaping = shaper("openai");
    let created_chunk = chunk_creator(&mut shaping, &mut decode_state, &chunk)
        .expect("the chunk shapes")
        .expect("a chunk is emitted");
    assert_eq!(
        first_choice(&created_chunk).finish_reason.as_deref(),
        Some("length")
    );
    assert!(shaping.progress.sent_last_chunk);
}

#[test]
fn creator_repetition_limit_is_honoured() {
    let mut decode_state = state("gpt-4");
    decode_state.repeated_chunk_limit = 2;
    let mut shaping = shaper("openai");
    let first = json_chunk(r#"{"id": "c1", "choices": [{"delta": {"content": "repeat"}}]}"#);
    chunk_creator(&mut shaping, &mut decode_state, &first).expect("the first chunk shapes");
    let second = json_chunk(r#"{"id": "c1", "choices": [{"delta": {"content": "repeat"}}]}"#);
    let error =
        chunk_creator(&mut shaping, &mut decode_state, &second).expect_err("the repetition fails");
    assert!(matches!(error, ProviderChunkError::Shape { .. }));
}

#[test]
fn replays_the_provider_stream_chunk_corpus() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/provider_stream_chunk");
    let entries: Vec<_> = fs::read_dir(root).expect("the corpus exists").collect();
    assert!(!entries.is_empty());
    entries
        .into_iter()
        .try_for_each(|entry| {
            let path = entry.expect("a corpus entry").path();
            let bytes = fs::read(&path)?;
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("the seed has a name");
            check_seed(name, decode_fuzzed_stream_chunk(&bytes));
            Ok::<(), Error>(())
        })
        .expect("every seed reads");
}

fn check_seed(name: &str, seed: Option<Result<DecodedChunk, ProviderChunkError>>) {
    let outcome = seed.expect("the seed has a provider byte");
    match name {
        "openai-text" => assert_eq!(outcome.expect("the seed decodes").text, "hi"),
        "generic-text" => assert_eq!(outcome.expect("the seed decodes").text, "generic"),
        "replicate-succeeded" => assert!(outcome.expect("the seed decodes").is_finished),
        "custom-openai" => assert!(outcome.expect("the seed decodes").early_return),
        "vertex-blocked" => assert!(matches!(
            outcome.expect_err("the seed fails"),
            ProviderChunkError::VertexBlocked
        )),
        "petals-text" => assert_eq!(
            outcome.expect("the seed decodes").text,
            "abcdefghijklmnopqrstuvwxyzabcd"
        ),
        "nlp-cloud-error" => assert!(matches!(
            outcome.expect_err("the seed fails"),
            ProviderChunkError::UnknownStream
        )),
        "openai-text-line" => {
            outcome.unwrap_err();
        }
        other => panic!("missing outcome for corpus input {other}"),
    }
}
