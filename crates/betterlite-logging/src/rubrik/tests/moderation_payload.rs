use betterlite_logging::rubrik::moderation_payload::{build_response_moderation, join_texts};
use betterlite_logging::rubrik::tool_calls::{ChatCompletionMessageToolCall, Function};
use rstest::rstest;

#[rstest]
#[case::no_segments(&[], "")]
#[case::single_segment(&["hello".to_owned()], "hello")]
#[case::drops_empty_segments(&["hello".to_owned(), String::new(), "world".to_owned()], "hello\nworld")]
fn join_texts_concatenates_non_empty_segments_with_newlines(
    #[case] texts: &[String],
    #[case] expected: &str,
) {
    assert_eq!(join_texts(texts), expected);
}

fn call(id: &str) -> ChatCompletionMessageToolCall {
    ChatCompletionMessageToolCall {
        id: id.to_owned(),
        kind: "function".to_owned(),
        function: Function {
            name: "lookup".to_owned(),
            arguments: "{}".to_owned(),
        },
    }
}

#[test]
fn response_payload_uses_null_content_and_stop_when_there_is_no_text_or_tool_calls() {
    let payload = build_response_moderation(&[], "", None, 1_700_000_000);
    assert_eq!(payload.model, "");
    assert_eq!(payload.object, "chat.completion");
    assert_eq!(payload.choices[0].finish_reason, "stop");
    assert_eq!(payload.choices[0].message.content, None);
    assert!(payload.choices[0].message.tool_calls.is_empty());
}

#[test]
fn response_payload_carries_content_and_tool_calls_with_finish_reason_tool_calls() {
    let tool_calls = [call("call_1")];
    let payload = build_response_moderation(&tool_calls, "hello", None, 1_700_000_000);
    assert_eq!(payload.choices[0].finish_reason, "tool_calls");
    assert_eq!(payload.choices[0].message.content, Some("hello".to_owned()));
    assert_eq!(payload.choices[0].message.tool_calls, tool_calls);
}

#[test]
fn response_payload_uses_the_given_request_id_when_present() {
    let payload = build_response_moderation(&[], "", Some("chatcmpl-known"), 1_700_000_000);
    assert_eq!(payload.id, "chatcmpl-known");
}

#[test]
fn response_payload_falls_back_to_a_generated_chatcmpl_id_when_request_id_is_absent() {
    let payload = build_response_moderation(&[], "", None, 1_700_000_000);
    assert!(payload.id.starts_with("chatcmpl-"));
    assert_eq!(payload.created, 1_700_000_000);
}
