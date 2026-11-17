use betterlite_models::decisions::Json;
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;

use super::super::chunk;
use super::fixtures::{field, json};

#[test]
fn chunk_helpers_read_the_first_choice_delta() {
    let choice_less = json(r#"{"id":"chat_a","choices":[]}"#);
    assert!(chunk::first_choice(&choice_less).is_none());
    assert!(chunk::delta(&choice_less).is_none());
    let no_delta = json(r#"{"id":"chat_a","choices":[{"index":0,"finish_reason":null}]}"#);
    assert!(chunk::first_choice(&no_delta).is_some());
    assert!(chunk::delta(&no_delta).is_none());
    let empty_delta = json(r#"{"id":"chat_a","choices":[{"index":0,"delta":{}}]}"#);
    assert!(chunk::delta(&empty_delta).is_some_and(indexmap::IndexMap::is_empty));
}

#[test]
fn chunk_helpers_read_only_the_first_choice() {
    let two_choices = json(
        r#"{"id":"chat_a","choices":[{"index":0,"delta":{"content":"first"}},{"index":1,"delta":{"content":"second"}}]}"#,
    );
    let delta = chunk::delta(&two_choices).expect("the first choice carries a delta");
    assert_eq!(
        delta.get("content"),
        Some(&json(r#""first""#)),
        "a later choice never reaches the stream"
    );
}

#[test]
fn chunk_view_folds_hidden_params_and_hoists_usage() {
    let raw = r#"{"id":"chat_a","created":0,"model":"gpt-4o-mini","choices":[],"usage":null,"hidden_params":{"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5},"additional_headers":{"x":"y"}}}"#;
    let parsed: StreamChunk = serde_json::from_str(raw).expect("the chunk decodes");
    let view = chunk::wrapper_view(&parsed).expect("the view builds");
    assert_eq!(field(&view, "id"), json(r#""chat_a""#));
    assert_eq!(
        field(&view, "hidden_params"),
        Json::Null,
        "the private key never reaches the view"
    );
    assert_eq!(
        field(&field(&view, "_hidden_params"), "additional_headers"),
        json(r#"{"x":"y"}"#)
    );
    assert_eq!(
        field(&view, "usage"),
        json(r#"{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}"#),
        "hidden usage fills a missing usage field"
    );
}
