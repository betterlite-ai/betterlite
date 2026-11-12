#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use super::anthropic_stream_chunk_events;
use crate::guardrail_message_scope::StreamItem;

fn text_item(chunk: &str) -> StreamItem {
    StreamItem::Json(GuardrailValue::Text(chunk.to_owned()))
}

fn parsed_types(item: &StreamItem) -> Vec<String> {
    anthropic_stream_chunk_events(item)
        .iter()
        .map(|event| match event.get("type") {
            Some(GuardrailValue::Text(kind)) => kind.clone(),
            other => format!("{other:?}"),
        })
        .collect()
}

#[rstest]
#[case::data_prefix("data: {\"type\":\"a\"}", &["a"])]
#[case::no_space_after_prefix("data:{\"type\":\"a\"}", &["a"])]
#[case::multiline_blocks("event: x\ndata: {\"type\":\"a\"}\n\nevent: y\ndata: {\"type\":\"b\"}\n\n", &["a", "b"])]
#[case::lines_in_one_block("data: {\"type\":\"a\"}\ndata: {\"type\":\"b\"}", &["a", "b"])]
#[case::carriage_return_line_breaks("data: {\"type\":\"a\"}\r\n\r\ndata: {\"type\":\"b\"}", &["a", "b"])]
#[case::bare_carriage_return("data: {\"type\":\"a\"}\rdata: {\"type\":\"b\"}", &["a", "b"])]
#[case::unicode_line_separator("data: {\"type\":\"a\"}\u{2028}data: {\"type\":\"b\"}", &["a", "b"])]
#[case::next_line_separator("data: {\"type\":\"a\"}\u{85}data: {\"type\":\"b\"}", &["a", "b"])]
#[case::file_separator("data: {\"type\":\"a\"}\u{1c}data: {\"type\":\"b\"}", &["a", "b"])]
#[case::unit_separator_is_stripped("\u{1f}data: {\"type\":\"a\"}\u{1f}", &["a"])]
#[case::padding_is_stripped("   data:   {\"type\":\"a\"}   ", &["a"])]
#[case::done_marker_is_skipped("data: [DONE]\ndata: {\"type\":\"a\"}", &["a"])]
#[case::empty_payload_is_skipped("data:\ndata:   \ndata: {\"type\":\"a\"}", &["a"])]
#[case::invalid_json_is_skipped("data: {nope\ndata: {\"type\":\"a\"}", &["a"])]
#[case::non_object_json_is_skipped("data: [1]\ndata: \"x\"\ndata: 5\ndata: null\ndata: {\"type\":\"a\"}", &["a"])]
#[case::lines_without_the_prefix_are_skipped("event: ping\nid: 1\nxdata: {\"type\":\"a\"}\n: comment", &[])]
#[case::prefix_is_case_sensitive("DATA: {\"type\":\"a\"}", &[])]
#[case::empty_chunk("", &[])]
fn text_chunks_yield_their_json_objects(#[case] chunk: &str, #[case] expected: &[&str]) {
    assert_eq!(parsed_types(&text_item(chunk)), expected);
}

#[test]
fn a_dict_item_is_its_own_event() {
    let event = serde_json::from_str::<GuardrailValue>(r#"{"type":"message_start"}"#).unwrap();
    let events = anthropic_stream_chunk_events(&StreamItem::Json(event.clone()));
    assert_eq!(events.len(), 1);
    assert_eq!(GuardrailValue::Object(events[0].clone()), event);
}

#[rstest]
#[case::number(GuardrailValue::SignedInteger(1))]
#[case::array(GuardrailValue::Array(vec![GuardrailValue::Text("data: {}".to_owned())]))]
#[case::null(GuardrailValue::Null)]
#[case::boolean(GuardrailValue::Boolean(true))]
fn other_json_items_yield_nothing(#[case] item: GuardrailValue) {
    assert!(anthropic_stream_chunk_events(&StreamItem::Json(item)).is_empty());
}

#[test]
fn bytes_are_decoded_as_text() {
    let item = StreamItem::Bytes(b"event: x\ndata: {\"type\":\"a\"}\n\n".to_vec());
    assert_eq!(parsed_types(&item), ["a"]);
}

#[test]
fn invalid_utf8_becomes_a_replacement_character() {
    let item = StreamItem::Bytes(b"data: {\"type\":\"\xff\xfe\"}".to_vec());
    assert_eq!(parsed_types(&item), ["\u{FFFD}\u{FFFD}"]);
}

#[test]
fn a_truncated_sequence_is_one_replacement_character() {
    let item = StreamItem::Bytes(b"data: {\"type\":\"\xe2\x82\"}".to_vec());
    assert_eq!(parsed_types(&item), ["\u{FFFD}"]);
}

#[rstest]
fn replays_the_guardrail_message_scope_sse_chunk_corpus(
    #[files("fuzz/corpus/guardrail_message_scope_sse_chunk/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    drop(anthropic_stream_chunk_events(&StreamItem::Bytes(bytes)));
}

#[rstest]
#[case::message_start_and_delta("message_start_and_delta.txt", 2)]
#[case::broken_json_and_non_objects("broken_json_and_non_objects.txt", 1)]
fn the_corpus_files_parse_to_known_event_counts(#[case] file: &str, #[case] expected: usize) {
    let corpus =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/guardrail_message_scope_sse_chunk");
    let bytes = fs::read(corpus.join(file)).unwrap();
    assert_eq!(
        anthropic_stream_chunk_events(&StreamItem::Bytes(bytes)).len(),
        expected
    );
}
