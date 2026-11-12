#![cfg(test)]

use betterlite_openai::cometapi_wire::error::CometApiError;
use rstest::rstest;

use super::{CometApiChatIterator, CometApiChunk, CometApiIteratorError, StreamStep};

fn step(iterator: &mut CometApiChatIterator, line: &str) -> StreamStep {
    iterator.next_line(line).unwrap()
}

fn chunk(iterator: &mut CometApiChatIterator, line: &str) -> CometApiChunk {
    match step(iterator, line) {
        StreamStep::Chunk(found) => *found,
        other @ (StreamStep::Empty | StreamStep::Blank | StreamStep::Done) => {
            panic!("expected a chunk, got {other:?}")
        }
    }
}

fn api_error(failure: CometApiIteratorError) -> CometApiError {
    match failure {
        CometApiIteratorError::Api(error) => error,
        CometApiIteratorError::NotAnObject => panic!("expected a CometAPI error"),
    }
}

#[rstest]
#[case::empty("")]
#[case::spaces("   ")]
#[case::tab("\t")]
fn blank_lines_are_skipped(#[case] line: &str) {
    assert_eq!(
        step(&mut CometApiChatIterator::default(), line),
        StreamStep::Blank
    );
}

#[rstest]
#[case::bare("[DONE]")]
#[case::sse("data: [DONE]")]
#[case::padded("  data: [DONE]  ")]
#[case::inside_a_payload(r#"{"text":"a[DONE]b"}"#)]
fn the_done_marker_ends_the_stream(#[case] line: &str) {
    let mut iterator = CometApiChatIterator::default();
    assert_eq!(step(&mut iterator, line), StreamStep::Done);
    assert_eq!(
        step(
            &mut iterator,
            r#"{"id":"a","created":1,"model":"m","choices":[]}"#
        ),
        StreamStep::Done
    );
    assert_eq!(step(&mut iterator, "[DONE]"), StreamStep::Done);
}

#[rstest]
#[case::text("not json")]
#[case::empty_object("{}")]
#[case::null("null")]
#[case::boolean_false("false")]
#[case::zero("0")]
#[case::empty_text("\"\"")]
#[case::empty_list("[]")]
#[case::prefixed_text("data: not json")]
#[case::prefixed_empty_object("data:{}")]
fn an_unusable_line_is_the_base_empty_chunk(#[case] line: &str) {
    assert_eq!(
        step(&mut CometApiChatIterator::default(), line),
        StreamStep::Empty
    );
}

#[test]
fn a_parsed_chunk_goes_through_the_cometapi_parser() {
    let parsed = chunk(
        &mut CometApiChatIterator::default(),
        r#"data: {"id":"a","created":1,"model":"m","choices":[{"index":0,"delta":{"reasoning":"why","content":"hi"}}]}"#,
    );
    assert_eq!(parsed.id, "a");
    assert_eq!(parsed.object, "chat.completion.chunk");
    assert_eq!(parsed.created, 1);
    assert_eq!(parsed.model.as_deref(), Some("m"));
    let choice = serde_json::to_value(&parsed.choices[0]).unwrap();
    assert_eq!(choice["delta"]["reasoning"], "why");
    assert_eq!(choice["delta"]["reasoning_content"], "why");
    assert_eq!(choice["delta"]["content"], "hi");
}

#[test]
fn a_line_without_the_sse_prefix_still_parses() {
    let parsed = chunk(
        &mut CometApiChatIterator::default(),
        r#"{"id":"a","created":1,"model":"m","choices":[]}"#,
    );
    assert!(parsed.choices.is_empty());
}

#[test]
fn a_chunk_error_surfaces_as_the_cometapi_exception() {
    let mut iterator = CometApiChatIterator::default();
    let failure = iterator
        .next_line(r#"data: {"error":{"message":"boom","code":429}}"#)
        .unwrap_err();
    let error = api_error(failure);
    assert_eq!(error.message, "CometAPI Error: boom");
    assert_eq!(error.status_code, Some(429));
    assert_eq!(
        error.headers.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
}

#[test]
fn a_missing_chunk_field_surfaces_as_the_cometapi_exception() {
    let mut iterator = CometApiChatIterator::default();
    let failure = iterator
        .next_line(r#"data: {"id":"a","created":1,"model":"m"}"#)
        .unwrap_err();
    let error = api_error(failure);
    assert_eq!(error.status_code, Some(400));
}

#[rstest]
#[case::list("[1]")]
#[case::text("\"x\"")]
#[case::number("5")]
fn a_truthy_non_object_line_is_refused(#[case] line: &str) {
    let mut iterator = CometApiChatIterator::default();
    assert!(matches!(
        iterator.next_line(line),
        Err(CometApiIteratorError::NotAnObject)
    ));
}

#[test]
fn a_finish_reason_does_not_end_the_stream() {
    let mut iterator = CometApiChatIterator::default();
    let first = chunk(
        &mut iterator,
        r#"{"id":"a","created":1,"model":"m","choices":[{"index":0,"delta":{"content":"hi"},"finish_reason":"stop"}]}"#,
    );
    assert_eq!(first.id, "a");
    let second = chunk(
        &mut iterator,
        r#"{"id":"a","created":1,"model":"m","choices":[],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#,
    );
    assert!(second.usage.is_some());
}

#[test]
fn closing_the_iterator_ends_it() {
    let mut iterator = CometApiChatIterator::default();
    assert_eq!(iterator.close(), StreamStep::Done);
    assert_eq!(
        step(
            &mut iterator,
            r#"{"id":"a","created":1,"model":"m","choices":[]}"#
        ),
        StreamStep::Done
    );
}
