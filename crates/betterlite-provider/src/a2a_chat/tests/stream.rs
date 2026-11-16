use betterlite_streaming::stream_chunk_shaping::records::GenericChunk;
use rstest::rstest;

use crate::a2a_chat::error::A2aError;
use crate::a2a_chat::stream::chunk_parser;
use crate::ollama_chat::ids::RandomIds;

fn chunk_error(wire: &str) -> A2aError {
    chunk_parser(wire.as_bytes()).unwrap_err()
}

fn chunk(wire: &str) -> GenericChunk {
    chunk_parser(wire.as_bytes()).unwrap()
}

#[test]
fn a_completed_chunk_carries_text_and_finishes() {
    let parsed = chunk(
        r#"{"jsonrpc":"2.0","result":{"status":{"state":"completed"},"parts":[{"kind":"text","text":"done"}]}}"#,
    );
    assert_eq!(parsed.text, "done");
    assert!(parsed.is_finished);
    assert_eq!(parsed.finish_reason, "stop");
    assert_eq!(parsed.index, Some(0));
    assert_eq!(parsed.usage, None);
    assert_eq!(parsed.tool_use, None);
    assert_eq!(parsed.provider_specific_fields, None);
}

#[test]
fn a_failed_state_also_finishes() {
    let parsed = chunk(r#"{"result":{"status":{"state":"failed"},"parts":[]}}"#);
    assert_eq!(parsed.text, "");
    assert!(parsed.is_finished);
    assert_eq!(parsed.finish_reason, "stop");
}

#[test]
fn a_running_state_does_not_finish() {
    let parsed = chunk(
        r#"{"result":{"status":{"state":"running"},"parts":[{"kind":"text","text":"tick"}]}}"#,
    );
    assert_eq!(parsed.text, "tick");
    assert!(!parsed.is_finished);
    assert_eq!(parsed.finish_reason, "");
}

#[rstest]
#[case::done_flag(r#"{"done":true}"#, true)]
#[case::done_number(r#"{"done":1}"#, false)]
#[case::done_text(r#"{"done":"true"}"#, false)]
#[case::done_with_non_object_result(r#"{"result":"x","done":true}"#, true)]
#[case::state_with_non_object_status(r#"{"result":{"status":"completed"}}"#, false)]
fn only_an_exact_done_flag_finishes(#[case] wire: &str, #[case] finished: bool) {
    let parsed = chunk(wire);
    assert_eq!(parsed.is_finished, finished);
    assert_eq!(parsed.finish_reason == "stop", finished);
}

#[test]
fn a_plain_chunk_has_text_and_no_finish() {
    let parsed = chunk(r#"{"result":{"parts":[{"kind":"text","text":"plain"}]}}"#);
    assert_eq!(parsed.text, "plain");
    assert!(!parsed.is_finished);
    assert_eq!(parsed.finish_reason, "");
}

#[test]
fn an_error_object_is_a_500_a2a_error() {
    let error = chunk_error(r#"{"jsonrpc":"2.0","error":{"code":1,"message":"boom"}}"#);
    assert_eq!(error.status_code, 500);
    assert_eq!(error.message, "A2A error: boom");
    assert!(error.headers.is_empty());
}

#[rstest]
#[case::empty_object(r#"{"error":{}}"#)]
#[case::null_message(r#"{"error":{"message":null}}"#)]
fn an_error_object_without_a_message_says_unknown(#[case] wire: &str) {
    let error = chunk_error(wire);
    assert_eq!(error.status_code, 500);
    assert_eq!(error.message, "A2A error: Unknown error");
}

#[test]
fn a_non_object_error_value_is_ignored() {
    let parsed = chunk(r#"{"error":"boom","result":{"parts":[{"kind":"text","text":"still"}]}}"#);
    assert_eq!(parsed.text, "still");
    assert!(!parsed.is_finished);
}

#[test]
fn an_unreadable_chunk_is_an_empty_unfinished_chunk() {
    let parsed = chunk(r#"{"result":{"parts":[{"kind":"text","text":7}]}}"#);
    assert_eq!(parsed.text, "");
    assert!(!parsed.is_finished);
    assert_eq!(parsed.finish_reason, "");
    assert_eq!(parsed.index, Some(0));
}

#[rstest]
#[case::list("[1]")]
#[case::text(r#""text""#)]
#[case::number("7")]
#[case::null("null")]
#[case::not_json("{")]
fn a_non_object_chunk_is_an_empty_chunk(#[case] wire: &str) {
    let parsed = chunk(wire);
    assert_eq!(parsed.text, "");
    assert!(!parsed.is_finished);
    assert_eq!(parsed.finish_reason, "");
}

#[test]
fn random_ids_differ() {
    let first = RandomIds.next_id();
    let second = RandomIds.next_id();
    assert_ne!(first, second);
    assert_eq!(first.len(), 36);
}
