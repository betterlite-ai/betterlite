#![cfg(test)]

use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use rstest::rstest;
use serde::Serialize;

use super::tool_call_arguments;

fn fields(raw: &str) -> Object {
    serde_json::from_str(raw).unwrap()
}

#[derive(Serialize)]
struct ResponseFunction {
    name: &'static str,
    arguments: &'static str,
}

#[derive(Serialize)]
struct ResponseToolCall {
    id: &'static str,
    #[serde(rename = "type")]
    kind: &'static str,
    function: ResponseFunction,
}

#[derive(Serialize)]
struct ResponseMessage {
    role: &'static str,
    content: Option<&'static str>,
    tool_calls: Option<Vec<ResponseToolCall>>,
    function_call: Option<ResponseFunction>,
}

#[rstest]
#[case::no_calls(r#"{"role": "user", "content": "hi"}"#, &[])]
#[case::one_tool_call(
    r#"{"tool_calls": [{"function": {"name": "f", "arguments": "{\"a\": 1}"}}]}"#,
    &[r#"{"a": 1}"#]
)]
#[case::several_tool_calls(
    r#"{"tool_calls": [{"function": {"arguments": "first"}}, {"function": {"arguments": "second"}}]}"#,
    &["first", "second"]
)]
#[case::legacy_function_call(r#"{"function_call": {"name": "f", "arguments": "legacy"}}"#, &["legacy"])]
#[case::both_fields(
    r#"{"tool_calls": [{"function": {"arguments": "modern"}}], "function_call": {"arguments": "legacy"}}"#,
    &["modern", "legacy"]
)]
#[case::arguments_keep_their_padding(r#"{"function_call": {"arguments": "  padded  "}}"#, &["  padded  "])]
#[case::blank_arguments(r#"{"tool_calls": [{"function": {"arguments": "   "}}]}"#, &[])]
#[case::non_string_arguments(r#"{"tool_calls": [{"function": {"arguments": {"a": 1}}}]}"#, &[])]
#[case::missing_function(r#"{"tool_calls": [{"id": "call-1"}, {"function": null}]}"#, &[])]
#[case::null_fields(r#"{"tool_calls": null, "function_call": null}"#, &[])]
#[case::empty_tool_calls(r#"{"tool_calls": []}"#, &[])]
#[case::skips_bad_entries_and_keeps_the_rest(
    r#"{"tool_calls": ["oops", {"function": {"arguments": ""}}, {"function": {"arguments": "kept"}}]}"#,
    &["kept"]
)]
fn tool_call_arguments_reads_request_style_messages(#[case] raw: &str, #[case] expected: &[&str]) {
    assert_eq!(tool_call_arguments(&fields(raw)), expected);
}

#[test]
fn tool_call_arguments_reads_a_request_message_with_assistant_tool_calls() {
    let message: Message = serde_json::from_str(
        r#"{"role": "assistant", "content": null, "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "send", "arguments": "{\"ssn\": \"123\"}"}}]}"#,
    )
    .unwrap();

    assert_eq!(
        tool_call_arguments(&message.provider_specific_fields),
        [r#"{"ssn": "123"}"#]
    );
}

#[test]
fn tool_call_arguments_reads_a_typed_response_message() {
    let response = ResponseMessage {
        role: "assistant",
        content: None,
        tool_calls: Some(vec![ResponseToolCall {
            id: "call-1",
            kind: "function",
            function: ResponseFunction {
                name: "lookup",
                arguments: r#"{"q": "secret"}"#,
            },
        }]),
        function_call: Some(ResponseFunction {
            name: "old",
            arguments: "legacy args",
        }),
    };
    let message: Object = serde_json::from_str(&serde_json::to_string(&response).unwrap()).unwrap();

    assert_eq!(
        tool_call_arguments(&message),
        [r#"{"q": "secret"}"#, "legacy args"]
    );
}
