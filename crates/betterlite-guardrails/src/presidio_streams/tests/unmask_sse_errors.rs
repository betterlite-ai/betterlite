#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use super::shared::tokens;
use crate::presidio_streams::coalesce::split_sse_preface;
use crate::presidio_streams::unmask_sse::unmask_sse_bytes_chunk;

#[rstest]
fn replays_the_presidio_streams_sse_corpus(
    #[files("fuzz/corpus/presidio_streams_sse/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    drop(unmask_sse_bytes_chunk(&bytes, &tokens()));
    let (preface, rest) = split_sse_preface(&bytes);
    let mut joined = preface.to_vec();
    joined.extend_from_slice(rest);
    assert_eq!(joined, bytes);
}

#[rstest]
#[case::done_marker("data: [DONE]\n\n")]
#[case::not_json("data: {not json\n\n")]
#[case::not_an_object(r#"data: ["<PERSON_1>"]"#)]
#[case::missing_delta(r#"data: {"type":"content_block_delta"}"#)]
#[case::delta_is_not_an_object(r#"data: {"type":"content_block_delta","delta":"<PERSON_1>"}"#)]
#[case::event_type_missing(r#"data: {"delta":{"type":"text_delta","text":"<PERSON_1>"}}"#)]
#[case::other_event_type(
    r#"data: {"type":"message_delta","delta":{"type":"text_delta","text":"<PERSON_1>"}}"#
)]
#[case::other_delta_type(r#"data: {"type":"content_block_delta","delta":{"type":"input_json_delta","text":"<PERSON_1>"}}"#)]
#[case::delta_type_missing(r#"data: {"type":"content_block_delta","delta":{"text":"<PERSON_1>"}}"#)]
#[case::text_is_not_a_string(
    r#"data: {"type":"content_block_delta","delta":{"type":"text_delta","text":7}}"#
)]
#[case::no_space_after_data(
    r#"data:{"type":"content_block_delta","delta":{"type":"text_delta","text":"<PERSON_1>"}}"#
)]
#[case::not_a_data_line("id: <PERSON_1>\n\n")]
fn lines_that_do_not_qualify_are_returned_unchanged(#[case] chunk: &str) {
    assert_eq!(
        unmask_sse_bytes_chunk(chunk.as_bytes(), &tokens()),
        chunk.as_bytes()
    );
}

#[test]
fn a_chunk_that_is_not_utf8_is_returned_as_it_came() {
    let chunk = b"data: {\"type\":\"content_block_delta\"\xff}\r\n\r\n";
    assert_eq!(unmask_sse_bytes_chunk(chunk, &tokens()), chunk);
}

#[test]
fn a_bad_line_does_not_stop_the_good_lines_around_it() {
    let chunk = "data: {oops\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"<PERSON_1>\"}}";
    let expected = "data: {oops\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Alice\"}}";
    assert_eq!(
        unmask_sse_bytes_chunk(chunk.as_bytes(), &tokens()),
        expected.as_bytes()
    );
}
