#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::{empty_reply_error, field, final_text, finish_reason, message};

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn chat(message: &str) -> String {
    format!(r#"{{"choices":[{{"message":{message},"finish_reason":"stop"}}]}}"#)
}

#[rstest]
#[case::prose_only(
    r#"{"choices":[{"message":{"role":"assistant","content":"Hello there"},"finish_reason":"stop"}]}"#,
    "Hello there"
)]
#[case::prose_and_two_calls(
    r#"{"choices":[{"message":{"content":"Checking.","tool_calls":[{"id":"call_1","type":"function","function":{"name":"search","arguments":"{\"q\":\"weather\"}"}},{"id":"call_2","type":"function","function":{"name":"lookup","arguments":"{}"}}]},"finish_reason":"tool_calls"}]}"#,
    r#"Checking. [tool call] search({"q":"weather"}), [tool call] lookup({})"#
)]
#[case::calls_without_prose(
    r#"{"choices":[{"message":{"content":null,"tool_calls":[{"function":{"name":"search","arguments":"{}"}}]},"finish_reason":"tool_calls"}]}"#,
    "[tool call] search({})"
)]
#[case::function_call_singleton(
    r#"{"choices":[{"message":{"content":null,"function_call":{"name":"legacy","arguments":"{\"x\":1}"}},"finish_reason":"function_call"}]}"#,
    r#"[tool call] legacy({"x":1})"#
)]
#[case::empty_tool_calls_falls_back_to_function_call(
    r#"{"choices":[{"message":{"content":null,"tool_calls":[],"function_call":{"name":"legacy","arguments":"{}"}}}]}"#,
    "[tool call] legacy({})"
)]
#[case::text_tool_calls_falls_back_to_function_call(
    r#"{"choices":[{"message":{"tool_calls":"ab","function_call":{"name":"legacy","arguments":"{}"}}}]}"#,
    "[tool call] legacy({})"
)]
#[case::object_tool_calls_falls_back_to_function_call(
    r#"{"choices":[{"message":{"tool_calls":{"one":{}},"function_call":{"name":"legacy","arguments":"{}"}}}]}"#,
    "[tool call] legacy({})"
)]
#[case::listed_calls_win_over_function_call(
    r#"{"choices":[{"message":{"content":null,"tool_calls":[{"function":{"name":"search","arguments":"{}"}}],"function_call":{"name":"legacy","arguments":"{}"}}}]}"#,
    "[tool call] search({})"
)]
#[case::custom_tool_call(
    r#"{"choices":[{"message":{"content":null,"tool_calls":[{"type":"custom","custom":{"name":"c","input":"raw"}}]}}]}"#,
    "[tool call] c(raw)"
)]
#[case::empty_function_payload_falls_back_to_custom(
    r#"{"choices":[{"message":{"tool_calls":[{"function":{},"custom":{"name":"c","input":"i"}}]}}]}"#,
    "[tool call] c(i)"
)]
#[case::missing_name_is_unnamed(
    r#"{"choices":[{"message":{"tool_calls":[{"function":{"arguments":"{}"}}]}}]}"#,
    "[tool call] unnamed({})"
)]
#[case::empty_name_is_unnamed(
    r#"{"choices":[{"message":{"tool_calls":[{"function":{"name":"","arguments":"{}"}}]}}]}"#,
    "[tool call] unnamed({})"
)]
#[case::empty_arguments_use_input(
    r#"{"choices":[{"message":{"tool_calls":[{"custom":{"name":"c","arguments":"","input":"IN"}}]}}]}"#,
    "[tool call] c(IN)"
)]
#[case::scalar_call_is_unnamed(
    r#"{"choices":[{"message":{"tool_calls":[5]}}]}"#,
    "[tool call] unnamed()"
)]
#[case::content_parts_join_with_spaces(
    r#"{"choices":[{"message":{"content":[{"type":"text","text":"a"},{"type":"text","text":"b"}]}}]}"#,
    "a b"
)]
#[case::number_content_is_empty(r#"{"choices":[{"message":{"content":5}}]}"#, "")]
#[case::no_choices("{}", "")]
#[case::empty_choices(r#"{"choices":[]}"#, "")]
#[case::choices_not_a_list(r#"{"choices":"ab"}"#, "")]
#[case::message_null(r#"{"choices":[{"message":null}]}"#, "")]
#[case::message_not_an_object(r#"{"choices":[{"message":"x"}]}"#, "")]
#[case::empty_prose_and_no_calls(r#"{"choices":[{"message":{"content":""}}]}"#, "")]
#[case::empty_object_tool_calls_are_absent(
    r#"{"choices":[{"message":{"content":"hi","tool_calls":{}}}]}"#,
    "hi"
)]
#[case::empty_function_call_is_absent(
    r#"{"choices":[{"message":{"content":"hi","function_call":{}}}]}"#,
    "hi"
)]
#[case::prose_only_keeps_its_whitespace(r#"{"choices":[{"message":{"content":" hi "}}]}"#, " hi ")]
#[case::prose_whitespace_is_trimmed_around_calls(
    r#"{"choices":[{"message":{"content":" hi ","tool_calls":[{"function":{"name":"f","arguments":"{}"}}]}}]}"#,
    "hi  [tool call] f({})"
)]
#[case::scalar_response("5", "")]
#[case::null_response("null", "")]
fn final_text_reads_prose_and_calls(#[case] response: &str, #[case] expected: &str) {
    assert_eq!(final_text(&json(response)), expected);
}

#[rstest]
#[case::stop(r#"{"choices":[{"finish_reason":"stop"}]}"#, "stop")]
#[case::length(r#"{"choices":[{"message":null,"finish_reason":"length"}]}"#, "length")]
#[case::empty_string(r#"{"choices":[{"finish_reason":""}]}"#, "unknown")]
#[case::null(r#"{"choices":[{"finish_reason":null}]}"#, "unknown")]
#[case::absent(r#"{"choices":[{"message":{"content":"x"}}]}"#, "unknown")]
#[case::number_is_stringified(r#"{"choices":[{"finish_reason":5}]}"#, "5")]
#[case::false_is_unknown(r#"{"choices":[{"finish_reason":false}]}"#, "unknown")]
#[case::true_is_stringified(r#"{"choices":[{"finish_reason":true}]}"#, "True")]
#[case::zero_is_unknown(r#"{"choices":[{"finish_reason":0}]}"#, "unknown")]
#[case::list_keeps_json_text(r#"{"choices":[{"finish_reason":["stop"]}]}"#, r#"["stop"]"#)]
#[case::empty_list_is_unknown(r#"{"choices":[{"finish_reason":[]}]}"#, "unknown")]
#[case::empty_object_is_unknown(r#"{"choices":[{"finish_reason":{}}]}"#, "unknown")]
#[case::no_choices(r#"{"choices":[]}"#, "unknown")]
fn finish_reason_reads_the_choice(#[case] response: &str, #[case] expected: &str) {
    assert_eq!(finish_reason(&json(response)), expected);
}

#[rstest]
#[case::model_unknown(
    r#"{"choices":[]}"#,
    "",
    "shadow router returned an empty response; finish_reason=unknown, model=unknown"
)]
#[case::model_known(
    r#"{"choices":[{"message":{"content":""},"finish_reason":"stop"}]}"#,
    "gpt-4o",
    "shadow router returned an empty response; finish_reason=stop, model=gpt-4o"
)]
#[case::finish_reason_from_choice(
    r#"{"choices":[{"message":{"content":null},"finish_reason":"length"}]}"#,
    "shadow",
    "shadow router returned an empty response; finish_reason=length, model=shadow"
)]
fn empty_reply_error_names_the_response(
    #[case] response: &str,
    #[case] routed_model: &str,
    #[case] expected: &str,
) {
    assert_eq!(empty_reply_error(&json(response), routed_model), expected);
}

#[test]
fn field_reads_object_keys_only() {
    let document = json(r#"{"message":{"content":"hi"},"flag":true,"none":null}"#);
    assert_eq!(
        field(&document, "message"),
        Some(&json(r#"{"content":"hi"}"#))
    );
    assert_eq!(field(&document, "none"), Some(&json("null")));
    assert!(field(&document, "missing").is_none());
    assert!(field(&json(r#""text""#), "content").is_none());
    assert!(field(&json("[1,2]"), "0").is_none());
    assert!(field(&json("5"), "content").is_none());
    assert!(field(&json("null"), "content").is_none());
}

#[test]
fn message_is_none_without_a_readable_message() {
    assert!(message(&json(r#"{"choices":[]}"#)).is_none());
    assert!(message(&json(r#"{"choices":[{"message":null}]}"#)).is_none());
    assert!(message(&json(r#"{"choices":"ab"}"#)).is_none());
    assert_eq!(
        message(&json(&chat(r#"{"content":"hi"}"#))),
        Some(&json(r#"{"content":"hi"}"#))
    );
}
