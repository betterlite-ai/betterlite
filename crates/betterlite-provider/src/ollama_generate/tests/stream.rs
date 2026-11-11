use rstest::rstest;

use super::support::SequenceIds;
use crate::ollama_chat::usage::Usage;
use crate::ollama_generate::stream::{
    GenerateDoneChunk, GenerateStreamChunk, GenerateStreamStep, OllamaGenerateIterator,
};

const NOW: i64 = 1_700_000_000;

fn parse(iterator: &mut OllamaGenerateIterator, line: &str) -> GenerateStreamStep {
    iterator
        .chunk_parser(line.as_bytes(), NOW, &SequenceIds::new())
        .unwrap()
}

fn chunk(iterator: &mut OllamaGenerateIterator, line: &str) -> GenerateStreamChunk {
    match parse(iterator, line) {
        GenerateStreamStep::Chunk(found) => found,
        GenerateStreamStep::Done(done) => panic!("expected a text chunk, got {done:?}"),
    }
}

fn done(iterator: &mut OllamaGenerateIterator, line: &str) -> GenerateDoneChunk {
    match parse(iterator, line) {
        GenerateStreamStep::Done(found) => found,
        GenerateStreamStep::Chunk(found) => panic!("expected a done chunk, got {found:?}"),
    }
}

#[test]
fn a_text_line_becomes_a_content_chunk() {
    let mut iterator = OllamaGenerateIterator::default();
    let found = chunk(&mut iterator, r#"{"response":"hello","done":false}"#);
    assert_eq!(found.id, "chatcmpl-id-0");
    assert_eq!(found.object, "chat.completion.chunk");
    assert_eq!(found.created, NOW);
    assert_eq!(found.model, None);
    assert_eq!(found.usage, None);
    assert_eq!(found.choices.len(), 1_usize);
    assert_eq!(found.choices[0].index, 0_u32);
    assert_eq!(found.choices[0].finish_reason, None);
    assert_eq!(found.choices[0].delta.content.as_deref(), Some("hello"));
    assert_eq!(found.choices[0].delta.reasoning_content, None);
    assert_eq!(found.choices[0].delta.tool_calls, None);
}

#[test]
fn a_whitespace_only_response_is_a_content_chunk() {
    let mut iterator = OllamaGenerateIterator::default();
    let found = chunk(&mut iterator, r#"{"response":"  ","done":false}"#);
    assert_eq!(found.choices[0].delta.content.as_deref(), Some("  "));
}

#[rstest]
#[case::open_tag(r#"{"response":"<think>why","done":false}"#, None, Some("why"))]
#[case::close_tag(r#"{"response":"answer</think>","done":false}"#, Some("answer"), None)]
#[case::empty_response(r#"{"response":"","done":false}"#, None, Some(""))]
#[case::null_response(r#"{"response":null,"done":false}"#, None, Some(""))]
#[case::thinking_field(
    r#"{"response":"","thinking":"held","done":false}"#,
    None,
    Some("held")
)]
#[case::thinking_null(r#"{"response":null,"thinking":null,"done":false}"#, None, Some(""))]
#[case::no_thinking_key(r#"{"response":"","done":false}"#, None, Some(""))]
fn a_falsy_response_carries_reasoning(
    #[case] line: &str,
    #[case] content: Option<&str>,
    #[case] reasoning: Option<&str>,
) {
    let mut iterator = OllamaGenerateIterator::default();
    let found = chunk(&mut iterator, line);
    assert_eq!(found.choices[0].delta.content.as_deref(), content);
    assert_eq!(
        found.choices[0].delta.reasoning_content.as_deref(),
        reasoning
    );
}

#[test]
fn the_reasoning_state_spans_chunks() {
    let mut iterator = OllamaGenerateIterator::default();
    let opened = chunk(&mut iterator, r#"{"response":"<think>why","done":false}"#);
    assert_eq!(
        opened.choices[0].delta.reasoning_content.as_deref(),
        Some("why")
    );
    let held = chunk(&mut iterator, r#"{"response":"more","done":false}"#);
    assert_eq!(
        held.choices[0].delta.reasoning_content.as_deref(),
        Some("more")
    );
    let closed = chunk(
        &mut iterator,
        r#"{"response":"answer</think>","done":false}"#,
    );
    assert_eq!(closed.choices[0].delta.content.as_deref(), Some("answer"));
    assert_eq!(closed.choices[0].delta.reasoning_content, None);
}

#[test]
fn an_open_tag_wins_over_a_close_tag_in_the_same_chunk() {
    let mut iterator = OllamaGenerateIterator::default();
    let found = chunk(
        &mut iterator,
        r#"{"response":"a<think>b</think>c","done":false}"#,
    );
    assert_eq!(
        found.choices[0].delta.reasoning_content.as_deref(),
        Some("ab</think>c")
    );
    assert_eq!(found.choices[0].delta.content, None);
    let closed = chunk(&mut iterator, r#"{"response":"end</think>","done":false}"#);
    assert_eq!(closed.choices[0].delta.content.as_deref(), Some("end"));
    let late = chunk(
        &mut iterator,
        r#"{"response":"a<think>b</think>c","done":false}"#,
    );
    assert_eq!(
        late.choices[0].delta.content.as_deref(),
        Some("ab</think>c")
    );
}

#[test]
fn a_done_line_carries_the_stop_reason_and_usage() {
    let mut iterator = OllamaGenerateIterator::default();
    let found = done(
        &mut iterator,
        r#"{"response":"x","done":true,"prompt_eval_count":2,"eval_count":3}"#,
    );
    assert!(found.is_finished);
    assert_eq!(found.text, "");
    assert_eq!(found.finish_reason, "stop");
    assert_eq!(
        found.usage,
        Some(Usage {
            prompt_tokens: 2,
            completion_tokens: 3,
            total_tokens: 5,
        })
    );
}

#[rstest]
#[case::no_counts(r#"{"response":"","done":true}"#)]
#[case::prompt_only(r#"{"response":"","done":true,"prompt_eval_count":2}"#)]
#[case::eval_only(r#"{"response":"","done":true,"eval_count":2}"#)]
#[case::null_counts(r#"{"done":true,"prompt_eval_count":null,"eval_count":null}"#)]
fn a_done_line_without_both_counts_has_no_usage(#[case] line: &str) {
    let mut iterator = OllamaGenerateIterator::default();
    let found = done(&mut iterator, line);
    assert_eq!(found.usage, None);
}

#[test]
fn a_done_line_zero_counts_still_carry_usage() {
    let mut iterator = OllamaGenerateIterator::default();
    let found = done(
        &mut iterator,
        r#"{"done":true,"prompt_eval_count":0,"eval_count":0}"#,
    );
    assert_eq!(
        found.usage,
        Some(Usage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
        })
    );
}

#[rstest]
#[case::missing_done(r#"{"response":"x"}"#)]
#[case::done_not_bool(r#"{"response":"x","done":1}"#)]
#[case::missing_response(r#"{"done":false}"#)]
#[case::response_not_text(r#"{"response":5,"done":false}"#)]
#[case::thinking_not_text(r#"{"response":"","thinking":5,"done":false}"#)]
#[case::count_not_number(r#"{"response":"x","done":true,"prompt_eval_count":"2","eval_count":3}"#)]
fn a_malformed_chunk_is_a_shape_error(#[case] line: &str) {
    let mut iterator = OllamaGenerateIterator::default();
    let error = iterator
        .chunk_parser(line.as_bytes(), NOW, &SequenceIds::new())
        .unwrap_err();
    assert_eq!(error.status(), None);
}

#[test]
fn a_done_line_ignores_a_mistyped_response() {
    let mut iterator = OllamaGenerateIterator::default();
    let found = done(&mut iterator, r#"{"response":5,"done":true}"#);
    assert_eq!(found.finish_reason, "stop");
}

#[rstest]
#[case::not_json("not json")]
#[case::blank("")]
#[case::array("[1, 2]")]
#[case::string(r#""text""#)]
fn an_unreadable_line_is_a_shape_error(#[case] line: &str) {
    let mut iterator = OllamaGenerateIterator::default();
    let error = iterator
        .chunk_parser(line.as_bytes(), NOW, &SequenceIds::new())
        .unwrap_err();
    assert_eq!(error.status(), None);
}

#[test]
fn an_error_line_is_a_server_error() {
    let mut iterator = OllamaGenerateIterator::default();
    let error = iterator
        .chunk_parser(br#"{"error":"model not found"}"#, NOW, &SequenceIds::new())
        .unwrap_err();
    assert_eq!(error.status(), Some(500));
    assert!(error.to_string().contains("model not found"));
}

#[test]
fn an_error_line_wins_over_a_done_line() {
    let mut iterator = OllamaGenerateIterator::default();
    let error = iterator
        .chunk_parser(
            br#"{"error":{"message":"boom"},"done":true}"#,
            NOW,
            &SequenceIds::new(),
        )
        .unwrap_err();
    assert_eq!(error.status(), Some(500));
}
