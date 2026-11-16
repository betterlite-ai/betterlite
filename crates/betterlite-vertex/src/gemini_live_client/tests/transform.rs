#![cfg(test)]

use betterlite_prompts::arguments::json::Json;
use rstest::rstest;

use crate::gemini_live_client::error::LiveClientError;
use crate::gemini_live_client::records::{MessageKind, message_kind};
use crate::gemini_live_client::transform::audio_mime_type;

use super::support::{Session, assert_messages, json, with_warnings};

const MANUAL: &str =
    r#"{"setup":{"realtimeInputConfig":{"automaticActivityDetection":{"disabled":true}}}}"#;
const MALFORMED: &str = r#"{"setup":{"realtimeInputConfig":{"automaticActivityDetection":{"disabled":true,"prefixPaddingMs":"bad"}}}}"#;
const AUTOMATIC: &str =
    r#"{"setup":{"realtimeInputConfig":{"automaticActivityDetection":{"disabled":false}}}}"#;

fn append(data: &str) -> String {
    format!(r#"{{"type":"input_audio_buffer.append","audio":"{data}"}}"#)
}

fn tool_reply(call_id: &str, output: &str) -> String {
    format!(
        r#"{{"type":"conversation.item.create","item":{{"type":"function_call_output","call_id":"{call_id}","output":{output}}}}}"#
    )
}

fn tool_reply_without_output(call_id: &str) -> String {
    format!(
        r#"{{"type":"conversation.item.create","item":{{"type":"function_call_output","call_id":"{call_id}"}}}}"#
    )
}

fn tool_reply_without_call_id(output: &str) -> String {
    format!(
        r#"{{"type":"conversation.item.create","item":{{"type":"function_call_output","output":{output}}}}}"#
    )
}

fn user_message(parts: &str) -> String {
    format!(
        r#"{{"type":"conversation.item.create","item":{{"type":"message","content":[{parts}]}}}}"#
    )
}

#[test]
fn an_append_forwards_the_audio_with_the_default_mime_type() {
    let mut session = Session::new();
    let messages = session.send(&append("QUJD"));
    assert_messages(
        &messages,
        &[r#"{"realtimeInput":{"audio":{"mimeType":"audio/pcm;rate=24000","data":"QUJD"}}}"#],
    );
    assert_eq!(session.bytes(), 3);
}

#[test]
fn invalid_base64_is_counted_and_forwarded_unchanged() {
    let mut session = Session::new();
    let messages = session.send(&append("!!!!"));
    assert_messages(
        &messages,
        &[r#"{"realtimeInput":{"audio":{"mimeType":"audio/pcm;rate=24000","data":"!!!!"}}}"#],
    );
    assert_eq!(session.bytes(), 3);
}

#[rstest]
#[case::padding_two("QQ==", 1)]
#[case::padding_one("QUI=", 2)]
#[case::no_padding("QUJD", 3)]
#[case::two_chars("QQ", 1)]
#[case::one_char("Q", 0)]
#[case::empty("", 0)]
#[case::lone_padding("=", 0)]
#[case::double_padding("==", 0)]
fn the_audio_estimate_removes_terminal_padding(#[case] data: &str, #[case] bytes: u64) {
    let mut session = Session::new();
    session.send(&append(data));
    assert_eq!(session.bytes(), bytes);
}

#[test]
fn audio_estimates_accumulate_across_appends() {
    let mut session = Session::new();
    session.send(&append("QQ"));
    session.send(&append("QUJD"));
    assert_eq!(session.bytes(), 4);
}

#[test]
fn a_null_append_forwards_null_and_counts_nothing() {
    let mut session = Session::new();
    let messages = session.send(r#"{"type":"input_audio_buffer.append","audio":null}"#);
    assert_messages(
        &messages,
        &[r#"{"realtimeInput":{"audio":{"mimeType":"audio/pcm;rate=24000","data":null}}}"#],
    );
    assert_eq!(session.bytes(), 0);
}

#[test]
fn an_append_without_audio_is_an_error() {
    let mut session = Session::new();
    assert!(matches!(
        session.try_send(r#"{"type":"input_audio_buffer.append"}"#),
        Err(LiveClientError::MissingAudio)
    ));
    assert_eq!(session.bytes(), 0);
}

#[test]
fn clearing_the_buffer_keeps_the_audio_estimate() {
    let mut session = Session::new();
    session.send(&append("QUJD"));
    assert!(
        session
            .send(r#"{"type":"input_audio_buffer.clear"}"#)
            .is_empty()
    );
    assert_eq!(session.bytes(), 3);
}

#[test]
fn response_create_and_unknown_events_produce_no_messages() {
    let mut session = Session::new();
    assert!(session.send(r#"{"type":"response.create"}"#).is_empty());
    assert!(session.send(r#"{"type":"foo.bar"}"#).is_empty());
    assert!(session.send(r#"{"x":1}"#).is_empty());
}

#[rstest]
#[case::commit_automatic(r#"{"type":"input_audio_buffer.commit"}"#, None, "audioStreamEnd")]
#[case::end_automatic(r#"{"type":"input_audio_buffer.end"}"#, None, "audioStreamEnd")]
#[case::commit_manual(r#"{"type":"input_audio_buffer.commit"}"#, Some(MANUAL), "activityEnd")]
#[case::end_manual(r#"{"type":"input_audio_buffer.end"}"#, Some(MANUAL), "activityEnd")]
#[case::malformed_sibling(
    r#"{"type":"input_audio_buffer.commit"}"#,
    Some(MALFORMED),
    "activityEnd"
)]
#[case::disabled_false(
    r#"{"type":"input_audio_buffer.commit"}"#,
    Some(AUTOMATIC),
    "audioStreamEnd"
)]
#[case::non_object_setup(
    r#"{"type":"input_audio_buffer.commit"}"#,
    Some(r#"{"setup":5}"#),
    "audioStreamEnd"
)]
#[case::setup_not_json(
    r#"{"type":"input_audio_buffer.commit"}"#,
    Some("not json"),
    "audioStreamEnd"
)]
#[case::empty_request(r#"{"type":"input_audio_buffer.commit"}"#, Some(""), "audioStreamEnd")]
fn a_commit_or_end_sends_one_turn_boundary(
    #[case] source: &str,
    #[case] request: Option<&str>,
    #[case] key: &str,
) {
    let mut session = Session::new();
    let messages = session.send_with_request(source, request);
    let expected = format!(r#"{{"realtimeInput":{{"{key}":true}}}}"#);
    assert_messages(&messages, &[expected.as_str()]);
}

#[rstest]
#[case::disabled_string(
    r#"{"setup":{"realtimeInputConfig":{"automaticActivityDetection":{"disabled":"true"}}}}"#
)]
#[case::disabled_number(
    r#"{"setup":{"realtimeInputConfig":{"automaticActivityDetection":{"disabled":1}}}}"#
)]
#[case::detection_not_object(
    r#"{"setup":{"realtimeInputConfig":{"automaticActivityDetection":true}}}"#
)]
fn only_a_literal_true_boolean_selects_the_manual_boundary(#[case] request: &str) {
    let mut session = Session::new();
    let messages =
        session.send_with_request(r#"{"type":"input_audio_buffer.commit"}"#, Some(request));
    assert_messages(&messages, &[r#"{"realtimeInput":{"audioStreamEnd":true}}"#]);
}

#[test]
fn user_text_becomes_one_complete_client_turn() {
    let mut session = Session::new();
    let messages = session.send(&user_message(
        r#"{"type":"input_text","text":"hello"},{"type":"input_text","text":""},{"type":"input_text","text":"world"}"#,
    ));
    assert_messages(
        &messages,
        &[
            r#"{"clientContent":{"turns":[{"role":"user","parts":[{"text":"hello world"}]}],"turnComplete":true}}"#,
        ],
    );
}

#[test]
fn a_user_message_without_text_produces_no_messages() {
    let mut session = Session::new();
    assert!(
        session
            .send(&user_message(r#"{"type":"input_audio"}"#))
            .is_empty()
    );
    assert!(
        session
            .send(r#"{"type":"conversation.item.create"}"#)
            .is_empty()
    );
}

#[test]
fn a_tool_reply_carries_the_known_name_and_the_call_id() {
    let mut session = Session::new();
    session.register("call_1", "get_weather");
    let messages = session.send(&tool_reply("call_1", r#""{\"a\":1}""#));
    assert_messages(
        &messages,
        &[
            r#"{"toolResponse":{"functionResponses":[{"response":{"a":1},"id":"call_1","name":"get_weather"}]}}"#,
        ],
    );
}

#[rstest]
#[case::object(r#""{\"a\":1}""#, r#"{"a":1}"#)]
#[case::scalar(r#""5""#, r#"{"result":5}"#)]
#[case::list(r#""[1,2]""#, r#"{"result":[1,2]}"#)]
#[case::json_text(r#""\"x\"""#, r#"{"result":"x"}"#)]
#[case::malformed(r#""not json""#, r#"{"result":"not json"}"#)]
#[case::null("null", r#"{"result":null}"#)]
#[case::raw_object(r#"{"a":1}"#, r#"{"a":1}"#)]
#[case::raw_number("5", r#"{"result":5}"#)]
#[case::raw_list("[1,2]", r#"{"result":[1,2]}"#)]
#[case::raw_boolean("true", r#"{"result":true}"#)]
fn a_tool_output_is_parsed_or_wrapped_as_a_result(#[case] output: &str, #[case] response: &str) {
    let mut session = Session::new();
    session.register("call_1", "get_weather");
    let messages = session.send(&tool_reply("call_1", output));
    let expected = format!(
        r#"{{"toolResponse":{{"functionResponses":[{{"response":{response},"id":"call_1","name":"get_weather"}}]}}}}"#
    );
    assert_messages(&messages, &[expected.as_str()]);
}

#[test]
fn a_tool_reply_without_an_output_has_an_empty_response() {
    let mut session = Session::new();
    session.register("call_1", "get_weather");
    let messages = session.send(&tool_reply_without_output("call_1"));
    assert_messages(
        &messages,
        &[
            r#"{"toolResponse":{"functionResponses":[{"response":{},"id":"call_1","name":"get_weather"}]}}"#,
        ],
    );
}

#[test]
fn omitting_response_ids_drops_the_id_but_keeps_the_name() {
    let mut session = Session::without_response_ids();
    session.register("call_1", "get_weather");
    let messages = session.send(&tool_reply("call_1", r#""{}""#));
    assert_messages(
        &messages,
        &[r#"{"toolResponse":{"functionResponses":[{"response":{},"name":"get_weather"}]}}"#],
    );
}

#[test]
fn an_unknown_call_id_keeps_the_id_drops_the_name_and_warns() {
    let mut session = Session::new();
    let log = with_warnings(|| {
        let messages = session.send(&tool_reply("call_9", r#""{}""#));
        assert_messages(
            &messages,
            &[r#"{"toolResponse":{"functionResponses":[{"response":{},"id":"call_9"}]}}"#],
        );
    });
    assert!(log.contains("function name not found for call_id"));
}

#[test]
fn a_missing_or_empty_call_id_leaves_out_the_id() {
    let mut session = Session::new();
    let log = with_warnings(|| {
        let messages = session.send(&tool_reply_without_call_id(r#""true""#));
        assert_messages(
            &messages,
            &[r#"{"toolResponse":{"functionResponses":[{"response":{"result":true}}]}}"#],
        );
        let second = session.send(&tool_reply("", r#""{}""#));
        assert_messages(
            &second,
            &[r#"{"toolResponse":{"functionResponses":[{"response":{}}]}}"#],
        );
    });
    assert_eq!(
        log.matches("function name not found for call_id").count(),
        2
    );
}

#[test]
fn a_retried_reply_still_finds_the_name() {
    let mut session = Session::new();
    session.register("call_1", "get_weather");
    let first = session.send(&tool_reply("call_1", r#""{}""#));
    let second = session.send(&tool_reply("call_1", r#""{}""#));
    assert_eq!(first, second);
    assert!(
        session
            .state()
            .tool_call_names
            .entries
            .contains_key("call_1")
    );
}

#[test]
fn a_retried_reply_moves_the_call_id_to_the_recent_end() {
    let mut session = Session::new();
    session.register("call_1", "get_weather");
    session.register("call_2", "get_time");
    session.send(&tool_reply("call_1", r#""{}""#));
    let names: Vec<&str> = session
        .state()
        .tool_call_names
        .entries
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(names, ["call_2", "call_1"]);
}

#[test]
fn a_session_update_is_handed_to_the_setup_transformer() {
    let mut session = Session::new();
    let messages = session.send(
        r#"{"type":"session.update","session":{"instructions":"be brief","modalities":["text"]}}"#,
    );
    assert_eq!(messages.len(), 1);
    let Json::Object(message) = json(&messages[0]) else {
        panic!("the setup message is a JSON object");
    };
    assert!(message.contains_key("setup"));
}

#[rstest]
#[case::setup(r#"{"setup":{}}"#, MessageKind::Setup)]
#[case::setup_wins(r#"{"setup":{},"clientContent":{}}"#, MessageKind::Setup)]
#[case::realtime_input(r#"{"realtimeInput":{}}"#, MessageKind::Content)]
#[case::client_content(r#"{"clientContent":{}}"#, MessageKind::Content)]
#[case::tool_response(r#"{"toolResponse":{}}"#, MessageKind::Content)]
#[case::other(r#"{"serverContent":{}}"#, MessageKind::Other)]
#[case::empty("{}", MessageKind::Other)]
fn a_provider_message_is_setup_content_or_other(#[case] source: &str, #[case] kind: MessageKind) {
    let Json::Object(message) = json(source) else {
        panic!("the provider message is a JSON object");
    };
    assert_eq!(message_kind(&message), kind);
}

#[rstest]
#[case::pcm16("pcm16", "audio/pcm;rate=24000")]
#[case::ulaw("g711_ulaw", "audio/pcmu")]
#[case::alaw("g711_alaw", "audio/pcma")]
#[case::fallback("opus", "application/octet-stream")]
fn audio_mime_types_follow_the_input_format(#[case] format: &str, #[case] mime: &str) {
    assert_eq!(audio_mime_type(format), mime);
}
