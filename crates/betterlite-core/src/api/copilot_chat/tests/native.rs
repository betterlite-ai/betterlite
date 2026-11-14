use betterlite_models::decisions::Json;

use crate::api::copilot_chat::error::CopilotChatError;
use crate::api::copilot_chat::native_reply::{NativeReply, transform_parsed_response};

fn repair(text: &str) -> Json {
    let reply: NativeReply = serde_json::from_str(text).unwrap();
    let repaired = transform_parsed_response(reply).unwrap();
    serde_json::from_str(&serde_json::to_string(&repaired).unwrap()).unwrap()
}

fn field(value: &Json, name: &str) -> Option<Json> {
    let Json::Object(fields) = value else {
        panic!("not an object: {value:?}");
    };
    fields.get(name).cloned()
}

fn first_choice(text: &str) -> Json {
    let Some(Json::List(mut choices)) = field(&repair(text), "choices") else {
        panic!("no choices list");
    };
    choices.remove(0)
}

fn expected(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

#[rstest::rstest]
#[case::end_turn(
    r#"{"content":[{"type":"text","text":"hi"}],"stop_reason":"end_turn"}"#,
    "stop"
)]
#[case::stop_sequence(
    r#"{"content":[{"type":"text","text":"hi"}],"stop_reason":"stop_sequence"}"#,
    "stop"
)]
#[case::end_turn_without_text(r#"{"content":[],"stop_reason":"end_turn"}"#, "stop")]
#[case::stop_sequence_without_text(r#"{"content":[],"stop_reason":"stop_sequence"}"#, "stop")]
#[case::max_tokens(
    r#"{"content":[{"type":"text","text":"hi"}],"stop_reason":"max_tokens"}"#,
    "length"
)]
#[case::tool_use_stop_reason(
    r#"{"content":[{"type":"text","text":"hi"}],"stop_reason":"tool_use"}"#,
    "tool_calls"
)]
#[case::tool_blocks_beat_max_tokens(r#"{"content":[{"type":"tool_use","id":"t","name":"f","input":{}}],"stop_reason":"max_tokens"}"#, "tool_calls")]
#[case::unknown_reason_with_text(
    r#"{"content":[{"type":"text","text":"hi"}],"stop_reason":"refusal"}"#,
    "stop"
)]
#[case::unknown_reason_without_text(r#"{"content":[],"stop_reason":"refusal"}"#, "length")]
#[case::no_reason_with_text(r#"{"content":"hi"}"#, "stop")]
#[case::no_reason_without_text(r#"{"content":[]}"#, "length")]
#[case::no_content_key(r#"{"model":"claude"}"#, "length")]
fn repaired_finish_reason_follows_tools_then_stop_reason_then_text(
    #[case] raw: &str,
    #[case] reason: &str,
) {
    let choice = first_choice(raw);
    assert_eq!(
        field(&choice, "finish_reason"),
        Some(Json::Text(reason.to_owned()))
    );
    assert_eq!(field(&choice, "index"), Some(expected("0")));
}

#[test]
fn tool_only_reply_has_null_content_tool_calls_and_thinking_blocks() {
    let raw = r#"{"id":"msg_1","model":"claude-x","stop_reason":"tool_use","content":[
        {"type":"thinking","thinking":"hmm","signature":"s"},
        {"type":"tool_use","id":"toolu_1","name":"lookup","input":{"q":"x"}}]}"#;
    let wanted = r#"{"index":0,"finish_reason":"tool_calls","message":{
        "role":"assistant","content":null,
        "tool_calls":[{"id":"toolu_1","type":"function","index":1,
            "function":{"name":"lookup","arguments":"{\"q\": \"x\"}"}}],
        "thinking_blocks":[{"type":"thinking","thinking":"hmm","signature":"s"}]}}"#;
    assert_eq!(first_choice(raw), expected(wanted));
    assert_eq!(field(&repair(raw), "id"), Some(expected(r#""msg_1""#)));
    assert_eq!(
        field(&repair(raw), "model"),
        Some(expected(r#""claude-x""#))
    );
}

#[test]
fn text_and_tool_calls_together_keep_the_text() {
    let raw = r#"{"content":[{"type":"text","text":"a"},{"type":"text","text":"b"},
        {"type":"tool_use","id":"t","name":"f","input":{}}]}"#;
    let Some(message) = field(&first_choice(raw), "message") else {
        panic!("no message");
    };
    assert_eq!(field(&message, "content"), Some(expected(r#""ab""#)));
}

#[test]
fn empty_probe_reply_keeps_an_empty_string_without_tools_or_thinking() {
    let raw = r#"{"content":[],"usage":{"input_tokens":1,"output_tokens":1}}"#;
    let wanted =
        r#"{"index":0,"finish_reason":"length","message":{"role":"assistant","content":""}}"#;
    assert_eq!(first_choice(raw), expected(wanted));
}

#[test]
fn string_content_is_used_as_the_reply_text() {
    let wanted =
        r#"{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"plain"}}"#;
    assert_eq!(first_choice(r#"{"content":"plain"}"#), expected(wanted));
}

#[rstest::rstest]
#[case::absent(r#"{"content":"x"}"#)]
#[case::null(r#"{"content":"x","choices":null}"#)]
#[case::empty(r#"{"content":"x","choices":[]}"#)]
fn falsey_choices_are_repaired(#[case] raw: &str) {
    assert_eq!(
        field(&first_choice(raw), "finish_reason"),
        Some(expected(r#""stop""#))
    );
}

#[test]
fn filled_choices_are_left_alone() {
    let raw = r#"{"content":"x","choices":[{"index":3}],"usage":{"input_tokens":1}}"#;
    let reply: NativeReply = serde_json::from_str(raw).unwrap();
    assert!(reply.has_choices());
    assert_eq!(transform_parsed_response(reply.clone()).unwrap(), reply);
}

#[rstest::rstest]
#[case::fills_prompt_completion_and_total(
    r#"{"input_tokens":3,"output_tokens":4}"#,
    r#"{"input_tokens":3,"output_tokens":4,"prompt_tokens":3,"completion_tokens":4,"total_tokens":7}"#
)]
#[case::existing_counts_win(
    r#"{"prompt_tokens":1,"input_tokens":3,"completion_tokens":2,"output_tokens":4}"#,
    r#"{"prompt_tokens":1,"input_tokens":3,"completion_tokens":2,"output_tokens":4,"total_tokens":3}"#
)]
#[case::already_normalized(
    r#"{"prompt_tokens":1,"completion_tokens":2,"total_tokens":99,"cache":5}"#,
    r#"{"prompt_tokens":1,"completion_tokens":2,"total_tokens":99,"cache":5}"#
)]
#[case::only_input(
    r#"{"input_tokens":5}"#,
    r#"{"input_tokens":5,"prompt_tokens":5,"total_tokens":5}"#
)]
#[case::only_output(
    r#"{"output_tokens":6}"#,
    r#"{"output_tokens":6,"completion_tokens":6,"total_tokens":6}"#
)]
#[case::empty_usage(r#"{}"#, r#"{"total_tokens":0}"#)]
fn usage_is_normalized_without_overwriting_what_is_present(
    #[case] usage: &str,
    #[case] wanted: &str,
) {
    let repaired = repair(&format!(r#"{{"content":"x","usage":{usage}}}"#));
    assert_eq!(field(&repaired, "usage"), Some(expected(wanted)));
}

#[test]
fn usage_that_is_absent_or_null_is_not_normalized() {
    assert_eq!(field(&repair(r#"{"content":"x"}"#), "usage"), None);
    assert_eq!(
        field(&repair(r#"{"content":"x","usage":null}"#), "usage"),
        Some(Json::Null)
    );
}

#[test]
fn a_usage_count_that_is_not_an_integer_is_an_error() {
    let reply: NativeReply =
        serde_json::from_str(r#"{"content":"x","usage":{"prompt_tokens":"many"}}"#).unwrap();
    let error = transform_parsed_response(reply).unwrap_err();
    assert!(matches!(
        error,
        CopilotChatError::Usage {
            name: "prompt_tokens"
        }
    ));
}

#[test]
fn a_text_block_without_text_is_a_content_error() {
    let reply: NativeReply = serde_json::from_str(r#"{"content":[{"type":"text"}]}"#).unwrap();
    let error = transform_parsed_response(reply).unwrap_err();
    assert!(matches!(error, CopilotChatError::Content(_)));
}
