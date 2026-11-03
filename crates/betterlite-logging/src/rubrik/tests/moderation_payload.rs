use betterlite_logging::rubrik::moderation_payload::{
    ResponseModerationPayload, build_response_moderation, join_texts,
};
use betterlite_logging::rubrik::tool_calls::{ChatCompletionMessageToolCall, Function};
use betterlite_models::decisions::Json;
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

fn assert_payload_json(payload: &ResponseModerationPayload, expected: &str) {
    let serialized = serde_json::to_string(payload).expect("payload serializes");
    let actual_json = serde_json::from_str::<Json>(&serialized).expect("payload is JSON");
    let expected_json = serde_json::from_str::<Json>(expected).expect("expected value is JSON");
    assert_eq!(actual_json, expected_json);
}

#[rstest]
#[case::no_tool_calls(&[], "", r#"{"id":"chatcmpl-known","object":"chat.completion","created":1700000000,"model":"","choices":[{"index":0,"message":{"role":"assistant","content":null},"finish_reason":"stop"}]}"#)]
#[case::one_tool_call(&[call("call_1")], "hello", r#"{"id":"chatcmpl-known","object":"chat.completion","created":1700000000,"model":"","choices":[{"index":0,"message":{"role":"assistant","content":"hello","tool_calls":[{"id":"call_1","type":"function","function":{"name":"lookup","arguments":"{}"}}]},"finish_reason":"tool_calls"}]}"#)]
fn response_payload_serializes_to_the_after_completion_json(
    #[case] tool_calls: &[ChatCompletionMessageToolCall],
    #[case] content: &str,
    #[case] expected: &str,
) {
    let payload =
        build_response_moderation(tool_calls, content, Some("chatcmpl-known"), 1_700_000_000);
    assert_payload_json(&payload, expected);
}

#[test]
fn response_payload_uses_the_given_request_id_when_present() {
    let payload = build_response_moderation(&[], "", Some("chatcmpl-known"), 1_700_000_000);
    assert_eq!(payload.id, "chatcmpl-known");
}

#[rstest]
#[case::absent_request_id(None)]
#[case::empty_request_id(Some(""))]
fn response_payload_generates_a_chatcmpl_uuid4_id(#[case] request_id: Option<&str>) {
    let payload = build_response_moderation(&[], "", request_id, 1_700_000_000);
    let suffix = payload
        .id
        .strip_prefix("chatcmpl-")
        .expect("chat completion id");
    let uuid = uuid::Uuid::parse_str(suffix).expect("id suffix is a UUID");
    assert_eq!(uuid.get_version_num(), 4);
    assert_eq!(uuid.hyphenated().to_string(), suffix);
    let second_payload = build_response_moderation(&[], "", request_id, 1_700_000_000);
    assert_ne!(payload.id, second_payload.id);
    assert_eq!(payload.created, 1_700_000_000);
}
