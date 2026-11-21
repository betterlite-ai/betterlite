use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::fake::base;
use super::support::{
    array, chat_response, object, replay_chunk, run_buffered, stream_chunk, streaming_settings,
    text, unsigned,
};
use crate::guardrail_hooks::chunks::StreamChunk;
use crate::guardrail_hooks::errors::HookFailure;

fn chat_chunks() -> Vec<StreamChunk> {
    vec![
        StreamChunk::Json(stream_chunk("hello")),
        StreamChunk::Json(stream_chunk(" world")),
    ]
}

fn text_completion_chunks() -> Vec<StreamChunk> {
    vec![
        StreamChunk::Json(object(vec![
            ("id", text("cmpl-1")),
            ("model", text("gpt-3.5-turbo-instruct")),
            (
                "choices",
                array(vec![object(vec![
                    ("index", unsigned(0)),
                    ("text", text("Hello")),
                    ("finish_reason", GuardrailValue::Null),
                ])]),
            ),
        ])),
        StreamChunk::Json(object(vec![
            ("id", text("cmpl-1")),
            ("model", text("gpt-3.5-turbo-instruct")),
            (
                "choices",
                array(vec![object(vec![
                    ("index", unsigned(0)),
                    ("text", text(" world")),
                    ("finish_reason", text("stop")),
                ])]),
            ),
        ])),
    ]
}

#[tokio::test]
async fn a_text_completion_stream_passes_through_unscanned() {
    let runtime = base();
    let settings = streaming_settings();
    let given = text_completion_chunks();
    let output = run_buffered(&runtime, &settings, "/v1/chat/completions", given.clone())
        .await
        .unwrap();
    assert_eq!(output, given);
    assert!(runtime.scans.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_replayed_chunk_omits_the_null_fields_the_assembled_response_carries() {
    let mut runtime = base();
    runtime.assembled = Some(object(vec![
        ("id", GuardrailValue::Null),
        ("created", GuardrailValue::Null),
        ("usage", GuardrailValue::Null),
        ("model", text("gpt-test")),
        (
            "choices",
            array(vec![object(vec![
                ("index", unsigned(0)),
                (
                    "message",
                    object(vec![
                        ("role", text("assistant")),
                        ("content", text("hello")),
                        ("tool_calls", GuardrailValue::Null),
                    ]),
                ),
                ("finish_reason", GuardrailValue::Null),
            ])]),
        ),
    ]));
    runtime.scan_result = Ok(object(vec![]));
    let settings = streaming_settings();
    let output = run_buffered(&runtime, &settings, "/v1/chat/completions", chat_chunks())
        .await
        .unwrap();
    let expected = object(vec![
        ("object", text("chat.completion.chunk")),
        ("model", text("gpt-test")),
        (
            "choices",
            array(vec![object(vec![
                ("index", unsigned(0)),
                (
                    "delta",
                    object(vec![
                        ("role", text("assistant")),
                        ("content", text("hello")),
                    ]),
                ),
            ])]),
        ),
    ]);
    assert_eq!(output, vec![StreamChunk::Json(expected)]);
}

#[tokio::test]
async fn replays_the_assembled_response_as_one_chunk() {
    let mut runtime = base();
    runtime.assembled = Some(chat_response("gpt-test", text("hello world")));
    runtime.scan_result = Ok(object(vec![]));
    let settings = streaming_settings();
    let output = run_buffered(&runtime, &settings, "/v1/chat/completions", chat_chunks())
        .await
        .unwrap();
    assert_eq!(output, vec![StreamChunk::Json(replay_chunk("hello world"))]);
    assert_eq!(runtime.scans.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn replays_the_masked_text() {
    let mut runtime = base();
    runtime.assembled = Some(chat_response("gpt-test", text("hello world")));
    runtime.scan_result = Ok(object(vec![]));
    runtime.masked_response = Some(chat_response("gpt-test", text("[MASKED]")));
    let settings = streaming_settings();
    let output = run_buffered(&runtime, &settings, "/v1/chat/completions", chat_chunks())
        .await
        .unwrap();
    assert_eq!(output, vec![StreamChunk::Json(replay_chunk("[MASKED]"))]);
}

#[tokio::test]
async fn passes_the_chunks_through_when_the_stream_does_not_assemble() {
    let runtime = base();
    let settings = streaming_settings();
    let output = run_buffered(&runtime, &settings, "/v1/chat/completions", chat_chunks())
        .await
        .unwrap();
    assert_eq!(output, chat_chunks());
}

#[tokio::test]
async fn an_assembly_without_choices_fails_instead_of_dropping_the_stream() {
    let mut runtime = base();
    runtime.assembled = Some(object(vec![("model", text("gpt-test"))]));
    runtime.scan_result = Ok(object(vec![]));
    let settings = streaming_settings();
    let failure = run_buffered(&runtime, &settings, "/v1/chat/completions", chat_chunks())
        .await
        .unwrap_err();
    match failure {
        HookFailure::Failed { message } => {
            assert!(message.contains("could not be replayed"));
        }
        other @ (HookFailure::ModifyResponse(_) | HookFailure::Rejected { .. }) => {
            panic!("expected a failed replay, got {other:?}");
        }
    }
}

#[tokio::test]
async fn an_empty_choice_list_replays_as_a_chunk_without_choices() {
    let mut runtime = base();
    runtime.assembled = Some(object(vec![
        ("model", text("gpt-test")),
        ("choices", array(vec![])),
    ]));
    runtime.scan_result = Ok(object(vec![]));
    let settings = streaming_settings();
    let output = run_buffered(&runtime, &settings, "/v1/chat/completions", chat_chunks())
        .await
        .unwrap();
    assert_eq!(
        output,
        vec![StreamChunk::Json(object(vec![
            ("object", text("chat.completion.chunk")),
            ("model", text("gpt-test")),
            ("choices", array(vec![])),
        ]))]
    );
}

#[tokio::test]
async fn the_replayed_chunk_carries_the_assembled_identity_and_usage() {
    let mut runtime = base();
    runtime.assembled = Some(chat_response("gpt-test", text("hello")));
    runtime.scan_result = Ok(object(vec![]));
    let settings = streaming_settings();
    let output = run_buffered(&runtime, &settings, "/v1/chat/completions", chat_chunks())
        .await
        .unwrap();
    let StreamChunk::Json(GuardrailValue::Object(fields)) = &output[0] else {
        panic!("expected a json chunk");
    };
    assert_eq!(fields.get("id"), Some(&text("chatcmpl-1")));
    assert_eq!(
        fields.get("created"),
        Some(&GuardrailValue::UnsignedInteger(1))
    );
    assert!(fields.contains_key("usage"));
}
