#![cfg(test)]

use rstest::rstest;
use serde_json::value::RawValue;

use crate::cisco_conversation::json::Json;
use crate::cisco_conversation::request::{
    flatten_input, request_messages, tool_argument_parts, tool_definition_text,
};
use crate::cisco_conversation::response::{
    chat_messages, completion_messages, output_messages, reasoning_parts,
};
use crate::cisco_conversation::rewrite::{messages_to_input, rewrite_input_text};
use crate::cisco_conversation::{Fields, InspectMessage, normalize_content};

fn json(source: &str) -> Json {
    Json::parse(source)
}

fn fields(source: &str) -> Fields {
    serde_json::from_str(source).unwrap()
}

fn written<Output>(output: &Output) -> String
where
    Output: serde::Serialize,
{
    serde_json::to_string(output).unwrap()
}

#[rstest]
#[case::string(r#""hello""#, "hello")]
#[case::null("null", "")]
#[case::empty(r#""""#, "")]
#[case::integer("123", "123")]
#[case::boolean("true", "True")]
#[case::object(r#"{"a":[1,"b's",null]}"#, r#"{'a': [1, "b's", None]}"#)]
#[case::text_types(
    r#"[{"type":"text","text":"a"},{"type":"input_text","text":"b"},{"type":"output_text","text":"c"},{"type":"summary_text","text":"d"},{"type":"reasoning_text","text":"e"}]"#,
    "a b c d e"
)]
#[case::no_text_parts(r#"[1,"x",{"type":"image_url","text":"t"}]"#, "")]
#[case::number_text(r#"[{"type":"text","text":5}]"#, "5")]
#[case::empty_text_reads_content(r#"[{"type":"text","text":"","content":"c"}]"#, "c")]
#[case::nested_message(
    r#"[{"role":"user","content":[{"type":"input_text","text":"n"}]}]"#,
    "n"
)]
#[case::empty_arguments_still_join(
    r#"[{"type":"call","arguments":[{"type":"image"}],"output":"o"}]"#,
    " o"
)]
fn normalizes_content(#[case] content: &str, #[case] expected: &str) {
    assert_eq!(normalize_content(&json(content)), expected);
}

#[rstest]
#[case::both(
    r#"{"tool_calls":[{"function":{"arguments":"t"}}],"function_call":{"arguments":"f"}}"#,
    r#"["t","f"]"#
)]
#[case::nulls(r#"{"tool_calls":null,"function_call":null}"#, "[]")]
#[case::object_arguments(
    r#"{"tool_calls":[{"function":{"arguments":{"a":1}}}]}"#,
    r#"["{'a': 1}"]"#
)]
#[case::skipped(
    r#"{"tool_calls":["x",{"function":"y"},{"function":{"arguments":""}}]}"#,
    "[]"
)]
#[case::zero(r#"{"function_call":{"arguments":0}}"#, r#"["0"]"#)]
fn extracts_tool_arguments(#[case] message: &str, #[case] expected: &str) {
    assert_eq!(written(&tool_argument_parts(&fields(message))), expected);
}

#[rstest]
#[case::all(
    r#"{"reasoning_content":"r","thinking_blocks":[{"thinking":"t","reasoning":"","text":"x","data":"d"},"s"],"reasoning_items":[{"summary":[{"text":"s"}],"text":"i","reasoning":7,"reasoning_content":"c"}]}"#,
    r#"["r","t","x","s","i","c"]"#
)]
#[case::nulls(
    r#"{"reasoning_content":null,"thinking_blocks":null,"reasoning_items":null}"#,
    "[]"
)]
#[case::wrong_types(r#"{"reasoning_content":5,"thinking_blocks":"x"}"#, "[]")]
fn extracts_reasoning(#[case] message: &str, #[case] expected: &str) {
    assert_eq!(written(&reasoning_parts(&fields(message))), expected);
}

#[rstest]
#[case::string(r#""hello""#, r#"[{"role":"user","content":"hello"}]"#)]
#[case::empty_string(r#""""#, r#"[{"role":"user","content":""}]"#)]
#[case::null("null", "[]")]
#[case::integer("123", r#"[{"role":"user","content":"123"}]"#)]
#[case::object(r#"{"k":"v"}"#, r#"[{"role":"user","content":"{'k': 'v'}"}]"#)]
#[case::messages(
    r#"[{"role":"user","content":"x"},{"role":"assistant","content":[{"type":"output_text","text":"y"}]}]"#,
    r#"[{"role":"user","content":"x"},{"role":"assistant","content":"y"}]"#
)]
#[case::message_skips(
    r#"[{"role":null,"content":"x"},"skip",{"role":"user","content":""}]"#,
    r#"[{"role":"user","content":"x"}]"#
)]
#[case::flat_parts(
    r#"[{"type":"input_text","text":"a"},{"type":"output_text","text":"b"}]"#,
    r#"[{"role":"user","content":"a b"}]"#
)]
#[case::empty_list("[]", "[]")]
fn flattens_responses_input(#[case] input: &str, #[case] expected: &str) {
    assert_eq!(written(&flatten_input(&json(input))), expected);
}

#[rstest]
#[case::chat(
    r#"{"instructions":"sys","messages":[{"role":"user","content":"hi","tool_calls":[{"function":{"arguments":"a"}}]},{"role":"","content":"skip"},{"role":"assistant","content":""}],"tools":[{"type":"function","function":{"name":"n","description":"d"}}]}"#,
    r#"[{"role":"system","content":"sys"},{"role":"user","content":"hi a"},{"role":"system","content":"function n d"}]"#
)]
#[case::nulls(r#"{"instructions":null,"messages":null}"#, "[]")]
#[case::input(r#"{"input":"hello"}"#, r#"[{"role":"user","content":"hello"}]"#)]
#[case::empty_input_blocks_prompt(
    r#"{"input":"","prompt":"p"}"#,
    r#"[{"role":"user","content":""}]"#
)]
#[case::prompt(r#"{"prompt":"p"}"#, r#"[{"role":"user","content":"p"}]"#)]
#[case::null_prompt(r#"{"prompt":null}"#, "[]")]
#[case::instructions_win(
    r#"{"instructions":"i","prompt":"p"}"#,
    r#"[{"role":"system","content":"i"}]"#
)]
fn extracts_request_messages(#[case] data: &str, #[case] expected: &str) {
    assert_eq!(written(&request_messages(&fields(data))), expected);
}

#[rstest]
#[case::nested(
    r#"{"tools":[{"function":{"name":"a","parameters":{"properties":{"x":{"description":"deep","default":1}}}}}],"functions":[{"name":"f"}]}"#,
    "a deep f"
)]
#[case::missing("{}", "")]
fn flattens_tool_definitions(#[case] data: &str, #[case] expected: &str) {
    assert_eq!(tool_definition_text(&fields(data)), expected);
}

#[rstest]
#[case::chat(
    chat_messages,
    r#"{"choices":[{"message":{"content":"c","tool_calls":[{"function":{"arguments":"t"}}],"reasoning_content":"r"}},{"message":{"content":""}},{"index":2}]}"#,
    r#"[{"role":"assistant","content":"c t r"}]"#
)]
#[case::chat_null(chat_messages, r#"{"choices":null}"#, "[]")]
#[case::completion(
    completion_messages,
    r#"{"choices":[{"text":"a"},{"text":""},{"text":"b"}]}"#,
    r#"[{"role":"assistant","content":"a b"}]"#
)]
#[case::completion_null(completion_messages, r#"{"choices":null}"#, "[]")]
#[case::output(
    output_messages,
    r#"{"output":[{"content":[{"text":"t","reasoning":"r","thinking":"k"},"s"],"arguments":"a","text":"x","reasoning":"ignored"},"skip"]}"#,
    r#"[{"role":"assistant","content":"t r k a x"}]"#
)]
#[case::output_null(output_messages, r#"{"output":null}"#, "[]")]
fn extracts_response_messages(
    #[case] extract: fn(&Fields) -> Vec<InspectMessage>,
    #[case] response: &str,
    #[case] expected: &str,
) {
    assert_eq!(written(&extract(&fields(response))), expected);
}

#[rstest]
#[case::user(
    r#"[{"role":"user","content":"text"}]"#,
    r#"[{"role":"user","content":[{"type":"input_text","text":"text"}]}]"#
)]
#[case::assistant(
    r#"[{"role":"assistant","content":"text"}]"#,
    r#"[{"role":"assistant","content":[{"type":"output_text","text":"text"}]}]"#
)]
#[case::skips(
    r#"["x",{"role":"user"},{"content":[{"type":"input_text","text":"p"}]}]"#,
    r#"[{"role":"user","content":[{"type":"input_text","text":"p"}]}]"#
)]
#[case::nothing_usable(r#"["x",{"role":"user","content":5}]"#, "null")]
fn converts_messages_to_input(#[case] messages: &str, #[case] expected: &str) {
    let items: Vec<Box<RawValue>> = serde_json::from_str(messages).unwrap();
    assert_eq!(written(&messages_to_input(&items)), expected);
}

#[rstest]
#[case::string(r#""orig""#, r#""clean""#)]
#[case::null("null", "null")]
#[case::message_parts(
    r#"[{"role":"user","content":[{"type":"image"},{"type":"input_text","text":"x","id":1}]}]"#,
    r#"[{"role":"user","content":[{"type":"image"},{"type":"input_text","id":1,"text":"clean"}]}]"#
)]
#[case::message_string(
    r#"[{"role":"user","content":"x"},{"role":"assistant","content":"y"}]"#,
    r#"[{"role":"user","content":"clean"},{"role":"assistant","content":"y"}]"#
)]
#[case::earlier_user(
    r#"[{"role":"user","content":[{"type":"input_text","text":"a"}]},{"role":"user","content":[{"type":"image"}]}]"#,
    r#"[{"role":"user","content":[{"type":"input_text","text":"clean"}]},{"role":"user","content":[{"type":"image"}]}]"#
)]
#[case::no_user(r#"[{"role":"assistant","content":"x"}]"#, "null")]
#[case::flat_part(
    r#"[{"type":"input_text","text":"x"}]"#,
    r#"[{"type":"input_text","text":"clean"}]"#
)]
#[case::no_text_part(r#"[{"type":"image"}]"#, "null")]
fn rewrites_responses_input(#[case] original: &str, #[case] expected: &str) {
    assert_eq!(
        written(&rewrite_input_text(&json(original), "clean")),
        expected
    );
}
