#![cfg(test)]

use rstest::rstest;

use crate::gemini_live_client::decode::{Audio, ClientEvent, ItemCreate};
use crate::gemini_live_client::error::LiveClientError;

use super::support::{decode, decode_bytes};

fn tool_item(body: &str) -> String {
    format!(
        r#"{{"type":"conversation.item.create","item":{{"type":"function_call_output"{body}}}}}"#
    )
}

fn user_item(body: &str) -> String {
    format!(r#"{{"type":"conversation.item.create","item":{{"type":"message"{body}}}}}"#)
}

#[test]
fn a_session_update_without_a_session_is_the_default_update() {
    assert!(matches!(
        decode(r#"{"type":"session.update"}"#),
        ClientEvent::SessionUpdate(_)
    ));
    assert!(matches!(
        decode(r#"{"type":"session.update","session":null}"#),
        ClientEvent::SessionUpdate(_)
    ));
}

#[test]
fn a_session_update_carries_its_session() {
    let ClientEvent::SessionUpdate(update) = decode(
        r#"{"type":"session.update","session":{"instructions":"be brief","modalities":["text"]}}"#,
    ) else {
        panic!("the event is a session update");
    };
    assert_eq!(update.instructions.as_deref(), Some("be brief"));
    assert_eq!(update.modalities.as_deref(), Some(&["text".to_owned()][..]));
}

#[test]
fn the_buffer_and_response_events_decode_to_their_kinds() {
    assert!(matches!(
        decode(r#"{"type":"input_audio_buffer.commit"}"#),
        ClientEvent::AudioCommit
    ));
    assert!(matches!(
        decode(r#"{"type":"input_audio_buffer.end"}"#),
        ClientEvent::AudioEnd
    ));
    assert!(matches!(
        decode(r#"{"type":"input_audio_buffer.clear"}"#),
        ClientEvent::AudioClear
    ));
    assert!(matches!(
        decode(r#"{"type":"response.create"}"#),
        ClientEvent::ResponseCreate
    ));
}

#[rstest]
#[case::missing(r#"{"type":"input_audio_buffer.append"}"#, Audio::Missing)]
#[case::null(r#"{"type":"input_audio_buffer.append","audio":null}"#, Audio::Null)]
#[case::text(
    r#"{"type":"input_audio_buffer.append","audio":"QUJD"}"#,
    Audio::Data("QUJD".to_owned())
)]
fn an_append_keeps_missing_null_and_text_apart(#[case] source: &str, #[case] expected: Audio) {
    let ClientEvent::AudioAppend(audio) = decode(source) else {
        panic!("the event is an audio append");
    };
    assert_eq!(audio, expected);
}

#[rstest]
#[case::unknown_type(r#"{"type":"foo.bar"}"#)]
#[case::missing_type(r#"{"x":1}"#)]
#[case::null_type(r#"{"type":null}"#)]
#[case::non_string_type(r#"{"type":5}"#)]
fn an_event_without_a_known_type_is_unknown(#[case] source: &str) {
    assert!(matches!(decode(source), ClientEvent::Unknown));
}

#[rstest]
#[case::output_missing(r#","call_id":"call_1""#, Some("call_1"), None)]
#[case::output_null(r#","call_id":"call_1","output":null"#, Some("call_1"), Some(None))]
#[case::output_text(
    r#","call_id":"call_1","output":"{\"a\":1}""#,
    Some("call_1"),
    Some(Some("{\"a\":1}"))
)]
#[case::call_missing(r#","output":"5""#, None, Some(Some("5")))]
#[case::call_null(r#","call_id":null,"output":"5""#, None, Some(Some("5")))]
fn a_function_call_output_keeps_absent_and_null_values(
    #[case] body: &str,
    #[case] call_id: Option<&str>,
    #[case] output: Option<Option<&str>>,
) {
    let ClientEvent::ItemCreate(ItemCreate::ToolResult(result)) = decode(&tool_item(body)) else {
        panic!("the item is a tool result");
    };
    assert_eq!(result.call_id.as_deref(), call_id);
    assert_eq!(result.output.as_ref().map(|value| value.as_deref()), output);
}

#[rstest]
#[case::joined(
    r#","content":[{"type":"input_text","text":"hello"},{"type":"input_text","text":""},{"type":"input_text","text":null},{"type":"input_text","text":"world"},{"type":"input_audio"}]"#,
    "hello world"
)]
#[case::no_text(r#","content":[{"type":"input_audio"}]"#, "")]
#[case::null_text(r#","content":[{"type":"input_text","text":null}]"#, "")]
#[case::missing_content("", "")]
fn a_user_message_joins_its_truthy_text_parts(#[case] body: &str, #[case] text: &str) {
    let ClientEvent::ItemCreate(ItemCreate::UserText { text: actual }) = decode(&user_item(body))
    else {
        panic!("the item is user text");
    };
    assert_eq!(actual, text);
}

#[test]
fn a_missing_item_is_an_empty_user_message() {
    let ClientEvent::ItemCreate(ItemCreate::UserText { text }) =
        decode(r#"{"type":"conversation.item.create"}"#)
    else {
        panic!("the item is user text");
    };
    assert_eq!(text, "");
}

#[test]
fn invalid_json_reports_the_message_it_could_not_parse() {
    let error = decode_bytes(b"{\"type\":", usize::MAX).expect_err("invalid JSON fails");
    let LiveClientError::InvalidJson { message, .. } = error else {
        panic!("the error is InvalidJson");
    };
    assert_eq!(message, "{\"type\":");
}

#[rstest]
#[case::list(b"[]")]
#[case::string(b"\"hi\"")]
#[case::number(b"5")]
#[case::boolean(b"true")]
fn a_non_object_event_is_an_error(#[case] bytes: &[u8]) {
    assert!(matches!(
        decode_bytes(bytes, usize::MAX),
        Err(LiveClientError::NotAnObject(_))
    ));
}

#[rstest]
#[case::audio(r#"{"type":"input_audio_buffer.append","audio":5}"#, "audio")]
#[case::item(r#"{"type":"conversation.item.create","item":5}"#, "item")]
#[case::session(r#"{"type":"session.update","session":5}"#, "session")]
fn a_mistyped_field_names_its_path(#[case] source: &str, #[case] path: &'static str) {
    let error = decode_bytes(source.as_bytes(), usize::MAX).expect_err("the field is invalid");
    let LiveClientError::InvalidField { path: actual, .. } = error else {
        panic!("the error is InvalidField");
    };
    assert_eq!(actual, path);
}

#[test]
fn the_size_bound_counts_the_bytes_exactly() {
    let source = r#"{"type":"input_audio_buffer.commit"}"#;
    let limit = source.len();
    let too_small = limit.saturating_sub(1);
    assert!(decode_bytes(source.as_bytes(), limit).is_ok());
    let error = decode_bytes(source.as_bytes(), too_small).expect_err("the event is too large");
    assert!(
        matches!(error, LiveClientError::TooLarge { limit: reported } if reported == too_small)
    );
}

#[test]
fn an_oversized_event_is_refused_before_it_is_parsed() {
    assert!(matches!(
        decode_bytes(b"not json at all", 4),
        Err(LiveClientError::TooLarge { limit: 4 })
    ));
}
