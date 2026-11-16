#![cfg(test)]

use rstest::rstest;

use super::shared::tokens;
use crate::presidio_streams::unmask_sse::{PiiTokens, unmask_sse_bytes_chunk};

const PLAIN: &str =
    r#"data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"plain"}}"#;

#[rstest]
#[case::plain_delta(
    r#"data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"Hi <PERSON_1>"}}"#,
    r#"data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"Hi Alice"}}"#
)]
#[case::event_line_and_key_order_kept(
    "event: content_block_delta\ndata: {\"delta\":{\"text\":\"<PERSON_1>\",\"type\":\"text_delta\"},\"index\":0,\"type\":\"content_block_delta\"}",
    "event: content_block_delta\ndata: {\"delta\":{\"text\":\"Alice\",\"type\":\"text_delta\"},\"index\":0,\"type\":\"content_block_delta\"}"
)]
#[case::two_deltas_in_one_chunk(
    "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"<PERSON_1>\"}}\n\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"a <PERSON_1>\"}}",
    "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Alice\"}}\n\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"a Alice\"}}"
)]
#[case::text_without_a_token_is_untouched(PLAIN, PLAIN)]
#[case::carriage_returns_are_stripped_from_every_line(
    "event: x\r\ndata: {\"type\":\"ping\"}\r\n\r\n",
    "event: x\ndata: {\"type\":\"ping\"}\n\n"
)]
#[case::non_ascii_text_is_not_escaped(
    r#"data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"<PERSON_1> ü"}}"#,
    r#"data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"Alice ü"}}"#
)]
fn unmask_sse_delta_cases(#[case] chunk: &str, #[case] expected: &str) {
    let unmasked = unmask_sse_bytes_chunk(chunk.as_bytes(), &tokens());
    assert_eq!(String::from_utf8(unmasked).unwrap(), expected);
}

#[test]
fn unmask_sse_delta_without_tokens_changes_nothing() {
    let chunk =
        r#"data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"<PERSON_1>"}}"#;
    assert_eq!(
        unmask_sse_bytes_chunk(chunk.as_bytes(), &PiiTokens::new()),
        chunk.as_bytes()
    );
}
