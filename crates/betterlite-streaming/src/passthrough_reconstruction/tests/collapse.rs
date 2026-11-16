use crate::passthrough_reconstruction::collapse::collapse_pure_text_chunks;
use crate::passthrough_reconstruction::events::RawSseChunk;

use super::support::text_chunk;

const MESSAGE_START: &str = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"model\":\"claude-3-5-sonnet\"}}";
const TEXT_BLOCK_START: &str = "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}";
const BLOCK_STOP: &str =
    "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}";
const MESSAGE_STOP: &str = "event: message_stop\ndata: {\"type\":\"message_stop\"}";
const MERGED: &str = "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello world\"}}";

fn text_delta(index: i64, text: &str) -> String {
    format!(
        "event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":{index},\"delta\":{{\"type\":\"text_delta\",\"text\":\"{text}\"}}}}"
    )
}

fn pure_text_stream() -> Vec<RawSseChunk> {
    vec![text_chunk(&format!(
        "{MESSAGE_START}\n\n{TEXT_BLOCK_START}\n\nevent: ping\ndata: {{\"type\":\"ping\"}}\n\n{}\n\n{}\n\n{BLOCK_STOP}\n\nevent: message_delta\ndata: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"end_turn\"}},\"usage\":{{\"output_tokens\":5}}}}\n\n{MESSAGE_STOP}\n\n",
        text_delta(0, "Hello"),
        text_delta(0, " world"),
    ))]
}

#[test]
fn collapses_a_pure_text_run_into_one_delta() {
    let collapsed = collapse_pure_text_chunks(&pure_text_stream()).unwrap();
    assert_eq!(
        collapsed,
        Some(vec![
            MESSAGE_START.to_owned(),
            TEXT_BLOCK_START.to_owned(),
            MERGED.to_owned(),
            BLOCK_STOP.to_owned(),
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":5}}".to_owned(),
            MESSAGE_STOP.to_owned(),
        ])
    );
}

#[test]
fn collapses_deltas_split_across_chunks() {
    let chunks = [
        text_chunk(&format!("{MESSAGE_START}\n\n{TEXT_BLOCK_START}\n\n")),
        text_chunk(&format!("{}\n\n", text_delta(0, "Hello"))),
        text_chunk(&format!("{}\n\n{BLOCK_STOP}", text_delta(0, " world"))),
    ];
    let collapsed = collapse_pure_text_chunks(&chunks)
        .unwrap()
        .expect("pure text");
    assert_eq!(collapsed.get(2).map(String::as_str), Some(MERGED));
    assert_eq!(collapsed.len(), 4);
}

#[test]
fn accepts_byte_chunks() {
    let chunks = [RawSseChunk::Bytes(
        format!(
            "{MESSAGE_START}\n\n{TEXT_BLOCK_START}\n\n{}\n\n",
            text_delta(0, "x")
        )
        .into_bytes(),
    )];
    let collapsed = collapse_pure_text_chunks(&chunks)
        .unwrap()
        .expect("pure text");
    assert_eq!(collapsed.len(), 3);
}

#[rstest::rstest]
#[case::input_json("input_json_delta")]
#[case::thinking("thinking_delta")]
#[case::signature("signature_delta")]
#[case::citations("citations_delta")]
fn refuses_every_disallowed_delta_type(#[case] kind: &str) {
    let chunks = [text_chunk(&format!(
        "{TEXT_BLOCK_START}\n\nevent: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"{kind}\",\"text\":\"x\"}}}}"
    ))];
    assert_eq!(collapse_pure_text_chunks(&chunks).unwrap(), None, "{kind}");
}

#[test]
fn refuses_a_delta_that_is_not_text() {
    let chunks = [text_chunk(&format!(
        "{TEXT_BLOCK_START}\n\nevent: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"text\":\"x\"}}}}"
    ))];
    assert_eq!(collapse_pure_text_chunks(&chunks).unwrap(), None);
    let non_object = [text_chunk(&format!(
        "{TEXT_BLOCK_START}\n\nevent: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":[1]}}"
    ))];
    assert_eq!(collapse_pure_text_chunks(&non_object).unwrap(), None);
}

#[test]
fn refuses_a_block_that_is_not_text() {
    let chunks = [text_chunk(
        "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"x\",\"input\":{}}}",
    )];
    assert_eq!(collapse_pure_text_chunks(&chunks).unwrap(), None);
    let missing = [text_chunk(
        "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0}",
    )];
    assert_eq!(collapse_pure_text_chunks(&missing).unwrap(), None);
    let non_object = [text_chunk(
        "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":[1]}",
    )];
    assert_eq!(collapse_pure_text_chunks(&non_object).unwrap(), None);
}

#[test]
fn refuses_a_delta_for_a_block_that_never_started() {
    let chunks = [text_chunk(&text_delta(1, "x"))];
    assert_eq!(collapse_pure_text_chunks(&chunks).unwrap(), None);
}

#[test]
fn refuses_a_delta_whose_index_is_not_an_integer() {
    let chunks = [text_chunk(&format!(
        "{TEXT_BLOCK_START}\n\nevent: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":\"0\",\"delta\":{{\"type\":\"text_delta\",\"text\":\"x\"}}}}"
    ))];
    assert_eq!(collapse_pure_text_chunks(&chunks).unwrap(), None);
}

#[test]
fn refuses_interleaved_blocks() {
    let chunks = [text_chunk(&format!(
        "{TEXT_BLOCK_START}\n\nevent: content_block_start\ndata: {{\"type\":\"content_block_start\",\"index\":1,\"content_block\":{{\"type\":\"text\",\"text\":\"\"}}}}\n\n{}\n\n{}",
        text_delta(0, "a"),
        text_delta(1, "b"),
    ))];
    assert_eq!(collapse_pure_text_chunks(&chunks).unwrap(), None);
}

#[test]
fn refuses_malformed_and_non_object_data() {
    let malformed = [text_chunk("event: content_block_delta\ndata: {not json}")];
    assert_eq!(collapse_pure_text_chunks(&malformed).unwrap(), None);
    let non_object = [text_chunk("data: [1,2]")];
    assert_eq!(collapse_pure_text_chunks(&non_object).unwrap(), None);
    let broken_bytes = [RawSseChunk::Bytes(vec![0xff])];
    assert!(collapse_pure_text_chunks(&broken_bytes).is_err());
}

#[test]
fn refuses_a_stream_without_a_text_delta() {
    let chunks = [text_chunk(&format!("{MESSAGE_START}\n\n{MESSAGE_STOP}"))];
    assert_eq!(collapse_pure_text_chunks(&chunks).unwrap(), None);
    assert_eq!(collapse_pure_text_chunks(&[]).unwrap(), None);
}

#[test]
fn drops_interior_bare_markers_and_keeps_the_others() {
    let chunks = [text_chunk(&format!(
        "event: message_start\n\n{TEXT_BLOCK_START}\n\nevent: content_block_delta\n\nevent: ping\n\n{}\n\n{MESSAGE_STOP}",
        text_delta(0, "x"),
    ))];
    let collapsed = collapse_pure_text_chunks(&chunks)
        .unwrap()
        .expect("pure text");
    assert_eq!(
        collapsed,
        vec![
            "event: message_start".to_owned(),
            TEXT_BLOCK_START.to_owned(),
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"x\"}}".to_owned(),
            MESSAGE_STOP.to_owned(),
        ]
    );
}

#[test]
fn merges_under_index_zero_when_the_stream_carries_no_index() {
    let chunks = [text_chunk(
        "data: {\"type\":\"content_block_start\",\"content_block\":{\"type\":\"text\"}}\n\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"a\"}}\n\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\"}}",
    )];
    let collapsed = collapse_pure_text_chunks(&chunks)
        .unwrap()
        .expect("pure text");
    assert_eq!(
        collapsed.last().map(String::as_str),
        Some(
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"a\"}}"
        )
    );
}

#[test]
fn reads_a_non_string_delta_text_as_empty() {
    let chunks = [text_chunk(&format!(
        "{TEXT_BLOCK_START}\n\nevent: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":5}}}}"
    ))];
    let collapsed = collapse_pure_text_chunks(&chunks)
        .unwrap()
        .expect("a numeric text is read as absent");
    assert_eq!(
        collapsed.last().map(String::as_str),
        Some(
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"\"}}"
        )
    );
}
