#![cfg(test)]

use betterlite_openai::stream::chunk::Chunk;
use betterlite_prompts::arguments::json::Json;
use betterlite_streaming::stream_chunk_shaping::records::GenericChunk;

use crate::sagemaker_stream::error::SagemakerStreamError;
use crate::sagemaker_stream::project::{
    CHUNK_OBJECT, END_OF_TEXT, STOP_REASON, chat_chunk, token_chunk,
};

const MODEL: &str = "decoder-model";

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture is JSON")
}

fn chat(text: &str) -> Result<Chunk, SagemakerStreamError> {
    chat_chunk(MODEL, &json(text))
}

fn token(text: &str) -> Result<GenericChunk, SagemakerStreamError> {
    token_chunk(&json(text))
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
        "the SageMaker stream chunk does not have the chat chunk shape"
    );
}

#[test]
fn an_unknown_chunk_field_is_refused() {
    let error = chat(r#"{"id": "1", "created": 2, "choices": [], "extra": 1}"#).unwrap_err();

    assert!(matches!(error, SagemakerStreamError::Json(_)));
}

#[test]
fn a_non_object_messages_chunk_is_refused() {
    let error = chat("[1]").unwrap_err();

    assert!(matches!(error, SagemakerStreamError::NotObject));
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

    assert!(matches!(
        error,
        SagemakerStreamError::Field { field: "index" }
    ));
}

#[test]
fn a_float_index_is_refused() {
    let error = token(r#"{"index": 1.5}"#).unwrap_err();

    assert!(matches!(
        error,
        SagemakerStreamError::Field { field: "index" }
    ));
}

#[test]
fn a_list_token_is_refused() {
    let error = token(r#"{"token": []}"#).unwrap_err();

    assert!(matches!(
        error,
        SagemakerStreamError::Field { field: "token" }
    ));
}

#[test]
fn a_number_text_is_refused() {
    let error = token(r#"{"token": {"text": 5}}"#).unwrap_err();

    assert!(matches!(
        error,
        SagemakerStreamError::Field {
            field: "token.text"
        }
    ));
}

#[test]
fn a_non_object_token_is_refused() {
    let error = token("[1]").unwrap_err();

    assert!(matches!(error, SagemakerStreamError::NotObject));
}
