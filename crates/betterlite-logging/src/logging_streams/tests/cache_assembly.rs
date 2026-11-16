use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::{FakeChunks, json};
use crate::logging_streams::cache_assembly::assemble_complete_stream;
use crate::logging_success::types::SuccessResult;

fn finished(reason: &str) -> Json {
    json(&format!(
        r#"{{"id":"chatcmpl-1","object":"chat.completion.chunk","choices":[{{"index":0,"finish_reason":{reason}}}]}}"#
    ))
}

fn unfinished() -> Json {
    json(
        r#"{"id":"chatcmpl-1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":"hel"}}]}"#,
    )
}

fn returns_a_chat_response_without_buffering() {
    let response = json(r#"{"id":"chatcmpl-1","object":"chat.completion","choices":[]}"#);
    let result = SuccessResult::Chat(response.clone());
    let mut chunks = FakeChunks::new(None);
    let complete = assemble_complete_stream(&result, &unfinished(), &mut chunks);
    assert_eq!(complete, Some(response));
    assert!(chunks.pushes().is_empty());
    assert_eq!(chunks.build_calls(), 0);
}

fn buffers_a_chunk_without_a_finish_reason() {
    let chunk = unfinished();
    let mut chunks = FakeChunks::new(Some(finished("\"stop\"")));
    let complete =
        assemble_complete_stream(&SuccessResult::Chunk(chunk.clone()), &chunk, &mut chunks);
    assert_eq!(complete, None);
    assert_eq!(chunks.pushes(), vec![chunk]);
    assert_eq!(chunks.build_calls(), 0);
}

fn buffers_a_chunk_with_a_null_finish_reason() {
    let chunk = finished("null");
    let mut chunks = FakeChunks::new(Some(finished("\"stop\"")));
    let complete =
        assemble_complete_stream(&SuccessResult::Chunk(chunk.clone()), &chunk, &mut chunks);
    assert_eq!(complete, None);
    assert_eq!(chunks.pushes(), vec![chunk]);
}

fn builds_after_the_chunk_with_a_finish_reason() {
    let chunk = finished("\"stop\"");
    let built = json(
        r#"{"id":"chatcmpl-1","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"hello"},"finish_reason":"stop"}]}"#,
    );
    let mut chunks = FakeChunks::new(Some(built.clone()));
    let complete =
        assemble_complete_stream(&SuccessResult::Chunk(chunk.clone()), &chunk, &mut chunks);
    assert_eq!(complete, Some(built));
    assert_eq!(chunks.pushes(), vec![chunk]);
    assert_eq!(chunks.build_calls(), 1);
}

fn builds_after_an_empty_finish_reason() {
    let chunk = finished("\"\"");
    let built = json(r#"{"id":"chatcmpl-1","object":"chat.completion","choices":[]}"#);
    let mut chunks = FakeChunks::new(Some(built.clone()));
    let complete =
        assemble_complete_stream(&SuccessResult::Chunk(chunk.clone()), &chunk, &mut chunks);
    assert_eq!(complete, Some(built));
}

fn builds_after_any_finish_value() {
    let chunk = finished("0");
    let built = json(r#"{"id":"chatcmpl-1","object":"chat.completion","choices":[]}"#);
    let mut chunks = FakeChunks::new(Some(built.clone()));
    let complete =
        assemble_complete_stream(&SuccessResult::Chunk(chunk.clone()), &chunk, &mut chunks);
    assert_eq!(complete, Some(built));
    assert_eq!(chunks.pushes(), vec![chunk]);
    assert_eq!(chunks.build_calls(), 1);
}

fn keeps_a_builder_failure_out_of_the_cache() {
    let chunk = finished("\"stop\"");
    let mut chunks = FakeChunks::failing();
    let complete =
        assemble_complete_stream(&SuccessResult::Chunk(chunk.clone()), &chunk, &mut chunks);
    assert_eq!(complete, None);
    assert_eq!(chunks.pushes(), vec![chunk]);
    assert_eq!(chunks.build_calls(), 1);
}

fn buffers_a_chunk_with_empty_choices() {
    let chunk = json(r#"{"id":"chatcmpl-1","object":"chat.completion.chunk","choices":[]}"#);
    let mut chunks = FakeChunks::new(Some(finished("\"stop\"")));
    let complete =
        assemble_complete_stream(&SuccessResult::Chunk(chunk.clone()), &chunk, &mut chunks);
    assert_eq!(complete, None);
    assert_eq!(chunks.pushes(), vec![chunk]);
}

fn buffers_a_chunk_without_choices() {
    let chunk = json(r#"{"id":"chatcmpl-1","object":"chat.completion.chunk"}"#);
    let mut chunks = FakeChunks::new(Some(finished("\"stop\"")));
    let complete =
        assemble_complete_stream(&SuccessResult::Chunk(chunk.clone()), &chunk, &mut chunks);
    assert_eq!(complete, None);
    assert_eq!(chunks.pushes(), vec![chunk]);
}

fn buffers_a_chunk_with_a_non_object_choice() {
    let chunk = json(r#"{"choices":["raw"]}"#);
    let mut chunks = FakeChunks::new(Some(finished("\"stop\"")));
    let complete =
        assemble_complete_stream(&SuccessResult::Chunk(chunk.clone()), &chunk, &mut chunks);
    assert_eq!(complete, None);
    assert_eq!(chunks.pushes(), vec![chunk]);
}

fn buffers_a_chunk_that_is_not_an_object() {
    let chunk = Json::Null;
    let mut chunks = FakeChunks::new(Some(finished("\"stop\"")));
    let complete =
        assemble_complete_stream(&SuccessResult::Chunk(chunk.clone()), &chunk, &mut chunks);
    assert_eq!(complete, None);
    assert_eq!(chunks.pushes(), vec![chunk]);
}

fn buffers_an_untyped_result_without_building() {
    let chunk = finished("\"stop\"");
    let mut chunks = FakeChunks::new(Some(finished("\"stop\"")));
    let complete =
        assemble_complete_stream(&SuccessResult::Other(chunk.clone()), &chunk, &mut chunks);
    assert_eq!(complete, None);
    assert_eq!(chunks.pushes(), vec![chunk]);
}

fn buffers_the_chunk_separately_from_the_result() {
    let result = SuccessResult::Chunk(finished("\"stop\""));
    let chunk = unfinished();
    let mut chunks = FakeChunks::new(None);
    let complete = assemble_complete_stream(&result, &chunk, &mut chunks);
    assert_eq!(complete, None);
    assert_eq!(chunks.pushes(), vec![chunk]);
}

#[rstest]
#[case::cache_assembly_returns_a_chat_response_without_buffering(
    returns_a_chat_response_without_buffering
)]
#[case::cache_assembly_buffers_a_chunk_without_a_finish_reason(
    buffers_a_chunk_without_a_finish_reason
)]
#[case::cache_assembly_buffers_a_chunk_with_a_null_finish_reason(
    buffers_a_chunk_with_a_null_finish_reason
)]
#[case::cache_assembly_builds_after_a_finish_reason(builds_after_the_chunk_with_a_finish_reason)]
#[case::cache_assembly_builds_after_an_empty_finish_reason(builds_after_an_empty_finish_reason)]
#[case::cache_assembly_builds_after_any_finish_value(builds_after_any_finish_value)]
#[case::cache_assembly_keeps_a_builder_failure_out_of_the_cache(
    keeps_a_builder_failure_out_of_the_cache
)]
#[case::cache_assembly_buffers_a_chunk_with_empty_choices(buffers_a_chunk_with_empty_choices)]
#[case::cache_assembly_buffers_a_chunk_without_choices(buffers_a_chunk_without_choices)]
#[case::cache_assembly_buffers_a_chunk_with_a_non_object_choice(
    buffers_a_chunk_with_a_non_object_choice
)]
#[case::cache_assembly_buffers_a_chunk_that_is_not_an_object(buffers_a_chunk_that_is_not_an_object)]
#[case::cache_assembly_buffers_an_untyped_result_without_building(
    buffers_an_untyped_result_without_building
)]
#[case::cache_assembly_buffers_the_chunk_separately_from_the_result(
    buffers_the_chunk_separately_from_the_result
)]
fn logging_streams_behavior_tests_behavior(#[case] behavior: fn()) {
    behavior();
}
