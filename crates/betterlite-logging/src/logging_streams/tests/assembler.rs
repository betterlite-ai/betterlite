use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::{json, state};
use crate::logging_streams::assembler::ChatStreamAssembler;
use crate::logging_success::ports::StreamAssembler as _;
use crate::logging_success::types::{SuccessMode, SuccessResult};

fn chat_response() -> Json {
    json(
        r#"{"id":"chatcmpl-1","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"hello"},"finish_reason":"stop"}]}"#,
    )
}

fn text_completion_response() -> Json {
    json(
        r#"{"id":"cmpl-1","object":"text_completion","choices":[{"index":0,"text":"hello","finish_reason":"stop"}]}"#,
    )
}

fn stream_chunk() -> Json {
    json(
        r#"{"id":"chatcmpl-1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":"hello"}}]}"#,
    )
}

fn plain_object() -> Json {
    json(r#"{"id":"chatcmpl-1","object":"chat.completion"}"#)
}

fn assembled(result: &SuccessResult, stream: Option<bool>) -> Option<Json> {
    ChatStreamAssembler.assemble(&state(stream), result, SuccessMode::Sync)
}

fn test_returns_a_streamed_chat_result() {
    let response = chat_response();
    assert_eq!(
        assembled(&SuccessResult::Chat(response.clone()), Some(true)),
        Some(response)
    );
}

fn test_returns_a_streamed_text_completion() {
    let response = text_completion_response();
    assert_eq!(
        assembled(&SuccessResult::Other(response.clone()), Some(true)),
        Some(response)
    );
}

fn test_drops_a_streamed_chunk() {
    assert_eq!(
        assembled(&SuccessResult::Chunk(stream_chunk()), Some(true)),
        None
    );
}

fn test_drops_a_streamed_plain_object() {
    assert_eq!(
        assembled(&SuccessResult::Other(plain_object()), Some(true)),
        None
    );
}

fn test_drops_streamed_embeddings() {
    let response = json(r#"{"object":"list","data":[]}"#);
    assert_eq!(
        assembled(&SuccessResult::Embeddings(response), Some(true)),
        None
    );
}

fn test_drops_an_object_whose_object_field_is_not_a_name() {
    let response = json(r#"{"object":7}"#);
    assert_eq!(assembled(&SuccessResult::Other(response), Some(true)), None);
}

fn test_drops_a_non_object_result() {
    assert_eq!(
        assembled(&SuccessResult::Other(Json::Null), Some(true)),
        None
    );
}

fn test_drops_a_chat_result_without_the_stream_flag() {
    assert_eq!(assembled(&SuccessResult::Chat(chat_response()), None), None);
}

fn test_drops_a_text_completion_without_the_stream_flag() {
    assert_eq!(
        assembled(&SuccessResult::Other(text_completion_response()), None),
        None
    );
}

fn test_drops_a_chunk_without_the_stream_flag() {
    assert_eq!(assembled(&SuccessResult::Chunk(stream_chunk()), None), None);
}

fn test_drops_a_plain_object_without_the_stream_flag() {
    assert_eq!(assembled(&SuccessResult::Other(plain_object()), None), None);
}

fn test_drops_a_chat_result_with_stream_false() {
    assert_eq!(
        assembled(&SuccessResult::Chat(chat_response()), Some(false)),
        None
    );
}

fn test_drops_a_text_completion_with_stream_false() {
    assert_eq!(
        assembled(
            &SuccessResult::Other(text_completion_response()),
            Some(false)
        ),
        None
    );
}

fn test_drops_a_chunk_with_stream_false() {
    assert_eq!(
        assembled(&SuccessResult::Chunk(stream_chunk()), Some(false)),
        None
    );
}

fn test_drops_a_plain_object_with_stream_false() {
    assert_eq!(
        assembled(&SuccessResult::Other(plain_object()), Some(false)),
        None
    );
}

#[rstest]
#[case::chat_stream_assembler_returns_a_streamed_chat_result(test_returns_a_streamed_chat_result)]
#[case::chat_stream_assembler_returns_a_streamed_text_completion(
    test_returns_a_streamed_text_completion
)]
#[case::chat_stream_assembler_drops_a_streamed_chunk(test_drops_a_streamed_chunk)]
#[case::chat_stream_assembler_drops_a_streamed_plain_object(test_drops_a_streamed_plain_object)]
#[case::chat_stream_assembler_drops_streamed_embeddings(test_drops_streamed_embeddings)]
#[case::chat_stream_assembler_drops_an_unnamed_object_field(
    test_drops_an_object_whose_object_field_is_not_a_name
)]
#[case::chat_stream_assembler_drops_a_non_object_result(test_drops_a_non_object_result)]
#[case::chat_stream_assembler_drops_a_chat_result_without_the_stream_flag(
    test_drops_a_chat_result_without_the_stream_flag
)]
#[case::chat_stream_assembler_drops_a_text_completion_without_the_stream_flag(
    test_drops_a_text_completion_without_the_stream_flag
)]
#[case::chat_stream_assembler_drops_a_chunk_without_the_stream_flag(
    test_drops_a_chunk_without_the_stream_flag
)]
#[case::chat_stream_assembler_drops_a_plain_object_without_the_stream_flag(
    test_drops_a_plain_object_without_the_stream_flag
)]
#[case::chat_stream_assembler_drops_a_chat_result_with_stream_false(
    test_drops_a_chat_result_with_stream_false
)]
#[case::chat_stream_assembler_drops_a_text_completion_with_stream_false(
    test_drops_a_text_completion_with_stream_false
)]
#[case::chat_stream_assembler_drops_a_chunk_with_stream_false(test_drops_a_chunk_with_stream_false)]
#[case::chat_stream_assembler_drops_a_plain_object_with_stream_false(
    test_drops_a_plain_object_with_stream_false
)]
fn logging_streams_behavior_tests_behavior(#[case] behavior: fn()) {
    behavior();
}
