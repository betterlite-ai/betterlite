use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::fake::base;
use super::support::{
    array, chat_response, message, object, run_buffered, sse_frame, stream_chunk,
    streaming_settings, text, unsigned, usage,
};
use crate::guardrail_hooks::chunks::StreamChunk;
use crate::guardrail_hooks::errors::{HookFailure, ModifyFailure};

fn blocked(message: &str, model: Option<&str>, original: Option<GuardrailValue>) -> HookFailure {
    HookFailure::ModifyResponse(Box::new(ModifyFailure {
        message: message.to_owned(),
        model: model.map(ToOwned::to_owned),
        original_response: original,
    }))
}

fn raw_chunks() -> Vec<StreamChunk> {
    vec![
        sse_frame(
            "message_start",
            "{\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"model\":\"claude-test\"}}",
        ),
        sse_frame(
            "content_block_delta",
            "{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}",
        ),
    ]
}

fn chat_chunks() -> Vec<StreamChunk> {
    vec![StreamChunk::Json(stream_chunk("hello"))]
}

fn response_without_model() -> GuardrailValue {
    object(vec![
        ("usage", GuardrailValue::Null),
        (
            "choices",
            array(vec![object(vec![
                ("index", unsigned(0)),
                ("message", message("assistant", text("hello"))),
            ])]),
        ),
    ])
}

#[tokio::test]
async fn a_raw_block_keeps_the_assembled_model_and_response() {
    let mut runtime = base();
    let assembled = chat_response("claude-test", text("hello"));
    runtime.assembled = Some(assembled.clone());
    runtime.scan_result = Err(blocked("blocked", Some("guardrail-model"), None));
    runtime.block_frames = vec![b"event: message_start\ndata: {}\n\n".to_vec()];
    let settings = streaming_settings();
    let output = run_buffered(&runtime, &settings, "/v1/messages", raw_chunks())
        .await
        .unwrap();
    assert_eq!(
        output,
        vec![StreamChunk::Bytes(runtime.block_frames[0].clone())]
    );
    let records = runtime.blocked_sse.lock().unwrap();
    assert_eq!(records[0].message, "blocked");
    assert_eq!(records[0].model, "claude-test");
    assert_eq!(records[0].original_response, assembled);
    assert!(runtime.masked.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_raw_block_without_an_assembled_model_keeps_the_exception_model() {
    let mut runtime = base();
    runtime.assembled = Some(response_without_model());
    runtime.scan_result = Err(blocked("blocked", Some("guardrail-model"), None));
    let settings = streaming_settings();
    run_buffered(&runtime, &settings, "/v1/messages", raw_chunks())
        .await
        .unwrap();
    let records = runtime.blocked_sse.lock().unwrap();
    assert_eq!(records[0].model, "guardrail-model");
}

#[tokio::test]
async fn a_raw_block_keeps_an_original_response_the_scan_set() {
    let attached = object(vec![("usage", usage(9, 4))]);
    let mut runtime = base();
    runtime.assembled = Some(chat_response("claude-test", text("hello")));
    runtime.scan_result = Err(blocked("blocked", None, Some(attached.clone())));
    let settings = streaming_settings();
    run_buffered(&runtime, &settings, "/v1/messages", raw_chunks())
        .await
        .unwrap();
    let records = runtime.blocked_sse.lock().unwrap();
    assert_eq!(records[0].original_response, attached);
}

#[tokio::test]
async fn a_chat_block_preserves_the_upstream_usage_and_the_exception_model() {
    let mut runtime = base();
    runtime.assembled = Some(chat_response("gpt-test", text("hello")));
    runtime.scan_result = Err(blocked("blocked", Some("guardrail-model"), None));
    runtime.replacement_chunks = vec![StreamChunk::Json(stream_chunk("blocked"))];
    let settings = streaming_settings();
    let output = run_buffered(&runtime, &settings, "/v1/chat/completions", chat_chunks())
        .await
        .unwrap();
    assert_eq!(output, runtime.replacement_chunks);
    let records = runtime.blocked_chat.lock().unwrap();
    assert_eq!(records[0].message, "blocked");
    assert_eq!(records[0].model, Some("guardrail-model".to_owned()));
    assert_eq!(records[0].usage, Some(usage(3, 2)));
    assert!(runtime.masked.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_raw_block_with_an_empty_assembled_model_keeps_the_exception_model() {
    let mut runtime = base();
    runtime.assembled = Some(object(vec![
        ("model", text("")),
        ("choices", array(vec![])),
    ]));
    runtime.scan_result = Err(blocked("blocked", Some("guardrail-model"), None));
    let settings = streaming_settings();
    run_buffered(&runtime, &settings, "/v1/messages", raw_chunks())
        .await
        .unwrap();
    let records = runtime.blocked_sse.lock().unwrap();
    assert_eq!(records[0].model, "guardrail-model");
}

#[tokio::test]
async fn a_chat_block_without_upstream_usage_passes_none() {
    let mut runtime = base();
    runtime.assembled = Some(response_without_model());
    runtime.scan_result = Err(blocked("blocked", None, None));
    let settings = streaming_settings();
    run_buffered(&runtime, &settings, "/v1/chat/completions", chat_chunks())
        .await
        .unwrap();
    let records = runtime.blocked_chat.lock().unwrap();
    assert_eq!(records[0].usage, None);
}
