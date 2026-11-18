#![cfg(test)]

use betterlite_openai::stream::chunk::Chunk;
use betterlite_streaming::stream_chunk_shaping::records::GenericChunk;

use crate::sagemaker_stream::SagemakerChunk;
use crate::sagemaker_stream::error::SagemakerStreamError;
use crate::sagemaker_stream::project::{
    CHUNK_OBJECT, END_OF_TEXT, Projection, STOP_REASON, project,
};

const MODEL: &str = "decoder-model";

fn chat(text: &str) -> Result<Chunk, SagemakerStreamError> {
    match project(MODEL, true, text) {
        Ok(Projection::Ready(SagemakerChunk::Chat(chunk))) => Ok(chunk),
        Ok(Projection::Ready(SagemakerChunk::Token(_))) => {
            panic!("a messages event yields a chat chunk")
        }
        Ok(Projection::Pending) => panic!("the fixture is JSON"),
        Err(error) => Err(error),
    }
}

fn token(text: &str) -> Result<GenericChunk, SagemakerStreamError> {
    match project(MODEL, false, text) {
        Ok(Projection::Ready(SagemakerChunk::Token(chunk))) => Ok(chunk),
        Ok(Projection::Ready(SagemakerChunk::Chat(_))) => {
            panic!("a token event yields a token chunk")
        }
        Ok(Projection::Pending) => panic!("the fixture is JSON"),
        Err(error) => Err(error),
    }
}

#[test]
fn the_object_is_forced_and_the_model_defaults_to_the_decoder_one() {
    let chunk = chat(r#"{"id": "1", "created": 2, "choices": []}"#).unwrap();

    assert_eq!(chunk.object, CHUNK_OBJECT);
    assert_eq!(chunk.model, MODEL);
    assert_eq!(chunk.id, "1");
    assert_eq!(chunk.created, 2);
    assert_eq!(chunk.choices.len(), 0);
    assert!(chunk.usage.is_none());
}

#[test]
fn a_chunk_object_is_replaced_with_the_chunk_one() {
    let chunk = chat(
        r#"{"id": "1", "object": "chat.completion", "created": 2, "model": "own", "choices": []}"#,
    )
    .unwrap();

    assert_eq!(chunk.object, CHUNK_OBJECT);
    assert_eq!(chunk.model, "own");
}

#[test]
fn a_null_model_takes_the_decoder_model() {
    let chunk = chat(r#"{"id": "1", "created": 2, "model": null, "choices": []}"#).unwrap();

    assert_eq!(chunk.model, MODEL);
}

#[test]
fn a_messages_chunk_keeps_its_choices() {
    let chunk = chat(
        r#"{"id": "1", "created": 2, "choices": [{"index": 0, "delta": {"role": "assistant", "content": "hi"}, "finish_reason": null}]}"#,
    )
    .unwrap();

    assert_eq!(chunk.choices.len(), 1);
    assert_eq!(chunk.choices[0].index, 0);
    assert_eq!(chunk.choices[0].delta.role.as_deref(), Some("assistant"));
    assert_eq!(chunk.choices[0].delta.content.as_deref(), Some("hi"));
    assert_eq!(chunk.choices[0].finish_reason, None);
}

#[test]
fn a_chunk_without_choices_is_refused() {
    let error = chat(r#"{"id": "1", "created": 2}"#).unwrap_err();

    assert!(matches!(error, SagemakerStreamError::Json(_)));
    assert_eq!(
        error.to_string(),
        "the SageMaker stream chunk does not have the declared shape"
    );
}

// the OpenAI chunk models set extra="allow", so an extra key is read past instead of failing the chunk
#[test]
fn an_unknown_chunk_field_is_ignored() {
    let chunk = chat(r#"{"id": "1", "created": 2, "choices": [], "extra": 1}"#).unwrap();

    assert_eq!(chunk.id, "1");
    assert_eq!(chunk.choices.len(), 0);
}

#[test]
fn a_non_object_messages_chunk_is_refused() {
    let error = chat("[1]").unwrap_err();

    assert!(matches!(error, SagemakerStreamError::Json(_)));
}

#[test]
fn a_partial_object_is_pending() {
    assert!(matches!(
        project(MODEL, true, r#"{"id": "1", "choices": ["#),
        Ok(Projection::Pending)
    ));
}

// StreamingChoices.__init__ maps an alias through map_finish_reason before the chunk reaches a client
#[test]
fn an_alias_finish_reason_maps_to_stop() {
    let chunk =
        chat(r#"{"id": "1", "created": 2, "choices": [{"finish_reason": "eos_token"}]}"#).unwrap();

    assert_eq!(chunk.choices[0].finish_reason.as_deref(), Some(STOP_REASON));
}

// StreamingChoices defaults index to 0 and delta to an empty Delta when the choice omits them
#[test]
fn a_choice_without_index_or_delta_takes_the_defaults() {
    let chunk = chat(r#"{"id": "1", "created": 2, "choices": [{}]}"#).unwrap();
    let choice = &chunk.choices[0];

    assert_eq!(choice.index, 0);
    assert!(choice.delta.role.is_none());
    assert!(choice.delta.content.is_none());
    assert!(choice.delta.tool_calls.is_none());
}

// StreamingChoices stores None when the stop reason is falsy, so an empty one never reaches a client
#[test]
fn a_null_delta_and_an_empty_finish_reason_take_the_defaults() {
    let chunk =
        chat(r#"{"id": "1", "created": 2, "choices": [{"delta": null, "finish_reason": ""}]}"#)
            .unwrap();

    assert!(chunk.choices[0].delta.content.is_none());
    assert_eq!(chunk.choices[0].finish_reason, None);
}

// Delta.__init__ numbers a tool call without an index and types it function
#[test]
fn a_tool_call_without_index_or_type_takes_the_defaults() {
    let chunk = chat(
        r#"{"id": "1", "created": 2, "choices": [{"delta": {"tool_calls": [{"function": {"name": "f", "arguments": "{}"}}]}}]}"#,
    )
    .unwrap();
    let calls = chunk.choices[0].delta.tool_calls.as_ref().unwrap();

    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].index, 0);
    assert_eq!(calls[0].kind.as_deref(), Some("function"));
    assert_eq!(
        calls[0].function.as_ref().unwrap().name.as_deref(),
        Some("f")
    );
}

// the counter advances only for a call that had no index, so an explicit 7 is followed by 0
#[test]
fn an_explicit_tool_call_index_leaves_the_counter_alone() {
    let chunk = chat(
        r#"{"id": "1", "created": 2, "choices": [{"delta": {"tool_calls": [{"index": 7, "type": "custom", "function": {}}, {}]}}]}"#,
    )
    .unwrap();
    let calls = chunk.choices[0].delta.tool_calls.as_ref().unwrap();

    assert_eq!(calls[0].index, 7);
    assert_eq!(calls[0].kind.as_deref(), Some("custom"));
    assert_eq!(calls[1].index, 0);
    assert_eq!(calls[1].kind.as_deref(), Some("function"));
}

#[test]
fn a_missing_token_and_index_read_as_empty_and_zero() {
    let chunk = token("{}").unwrap();

    assert_eq!(chunk.text, "");
    assert_eq!(chunk.index, Some(0));
    assert!(!chunk.is_finished);
    assert_eq!(chunk.finish_reason, "");
    assert!(chunk.usage.is_none());
    assert!(chunk.tool_use.is_none());
    assert!(chunk.provider_specific_fields.is_none());
}

#[test]
fn a_null_token_reads_as_an_empty_one() {
    let chunk = token(r#"{"token": null}"#).unwrap();

    assert_eq!(chunk.text, "");
}

#[test]
fn a_null_text_reads_as_empty() {
    let chunk = token(r#"{"token": {"text": null}}"#).unwrap();

    assert_eq!(chunk.text, "");
    assert!(!chunk.is_finished);
}

#[test]
fn a_text_token_is_used_as_given() {
    let chunk = token(r#"{"token": {"text": "hello world"}, "index": 7}"#).unwrap();

    assert_eq!(chunk.text, "hello world");
    assert_eq!(chunk.index, Some(7));
    assert!(!chunk.is_finished);
    assert_eq!(chunk.finish_reason, "");
}

#[test]
fn the_end_of_text_token_finishes_with_stop() {
    let fixture = format!(r#"{{"token": {{"text": "{END_OF_TEXT}"}}, "index": 2}}"#);

    let chunk = token(&fixture).unwrap();
    assert_eq!(chunk.text, "");
    assert_eq!(chunk.index, Some(2));
    assert!(chunk.is_finished);
    assert_eq!(chunk.finish_reason, STOP_REASON);
    assert!(chunk.usage.is_none());
}

#[test]
fn an_index_is_used_as_given_negative_included() {
    let chunk = token(r#"{"index": -3}"#).unwrap();

    assert_eq!(chunk.index, Some(-3));
}

#[test]
fn a_zero_index_stays_zero() {
    let chunk = token(r#"{"token": {"text": "x"}, "index": 0}"#).unwrap();

    assert_eq!(chunk.index, Some(0));
    assert_eq!(chunk.text, "x");
}

#[test]
fn a_null_index_is_zero() {
    let chunk = token(r#"{"index": null}"#).unwrap();

    assert_eq!(chunk.index, Some(0));
}

#[test]
fn a_text_index_is_refused() {
    let error = token(r#"{"index": "first"}"#).unwrap_err();

    assert!(matches!(error, SagemakerStreamError::Json(_)));
}

#[test]
fn a_float_index_is_refused() {
    let error = token(r#"{"index": 1.5}"#).unwrap_err();

    assert!(matches!(error, SagemakerStreamError::Json(_)));
}

#[test]
fn a_list_token_is_refused() {
    let error = token(r#"{"token": []}"#).unwrap_err();

    assert!(matches!(error, SagemakerStreamError::Json(_)));
}

#[test]
fn a_number_text_is_refused() {
    let error = token(r#"{"token": {"text": 5}}"#).unwrap_err();

    assert!(matches!(error, SagemakerStreamError::Json(_)));
}

#[test]
fn a_non_object_token_is_refused() {
    let error = token("[1]").unwrap_err();

    assert!(matches!(error, SagemakerStreamError::Json(_)));
}
