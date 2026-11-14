#![cfg(test)]

use rstest::rstest;

use crate::model_armor::decisions::model_response::{
    CallResponse, ModelResponse, extract_content_from_response,
};

fn content_of(source: &str) -> String {
    let model_response: ModelResponse = serde_json::from_str(source).unwrap();
    extract_content_from_response(&CallResponse::Model(model_response)).unwrap()
}

#[test]
fn extract_content_from_response_joins_every_choice_content() {
    assert_eq!(
        content_of(
            r#"{"choices":[{"message":{"role":"assistant","content":"one"}},{"message":{"role":"assistant","content":"two"}}]}"#
        ),
        "onetwo"
    );
}

#[test]
fn extract_content_from_response_appends_each_tool_call_as_json() {
    assert_eq!(
        content_of(
            r#"{"choices":[{"message":{"content":"call:","tool_calls":[{"id":"1","type":"function","function":{"name":"f","arguments":"{}"}},{"id":"2","function":{"arguments":null}}]}}]}"#
        ),
        r#"call:{"function":{"arguments":"{}","name":"f"},"id":"1","type":"function"}{"function":{"arguments":"","name":null},"id":"2","type":"function"}"#
    );
}

#[test]
fn extract_content_from_response_appends_a_custom_tool_call_as_json() {
    assert_eq!(
        content_of(
            r#"{"choices":[{"message":{"content":null,"tool_calls":[{"id":"7","type":"custom","custom":{"name":"grep","input":"todo"}}]}}]}"#
        ),
        r#"{"id":"7","type":"custom","custom":{"name":"grep","input":"todo"}}"#
    );
}

#[test]
fn extract_content_from_response_appends_the_function_call_json() {
    assert_eq!(
        content_of(
            r#"{"choices":[{"message":{"content":"","function_call":{"name":"f","arguments":"{}"}}}]}"#
        ),
        r#"{"arguments":"{}","name":"f"}"#
    );
}

#[test]
fn extract_content_from_response_reads_a_streamed_delta() {
    assert_eq!(
        content_of(
            r#"{"choices":[{"delta":{"content":"partial"}},{"delta":{"content":null}},{"delta":{}}]}"#
        ),
        "partial"
    );
}

#[rstest]
#[case::null_function_call(r#"{"choices":[{"message":{"content":"hi","function_call":null}}]}"#)]
#[case::empty_tool_calls(r#"{"choices":[{"message":{"content":"hi","tool_calls":[]}}]}"#)]
#[case::null_tool_calls(r#"{"choices":[{"message":{"content":"hi","tool_calls":null}}]}"#)]
fn extract_content_from_response_skips_empty_or_null_calls(#[case] source: &str) {
    assert_eq!(content_of(source), "hi");
}

#[rstest]
#[case::null_content(r#"{"choices":[{"message":{"role":"assistant","content":null}}]}"#)]
#[case::null_message(r#"{"choices":[{"message":null}]}"#)]
#[case::no_message(r#"{"choices":[{"finish_reason":"stop"}]}"#)]
#[case::null_choices(r#"{"choices":null}"#)]
#[case::no_choices("{}")]
fn extract_content_from_response_reads_nothing_from_an_empty_model_response(#[case] source: &str) {
    assert_eq!(content_of(source), "");
}

#[test]
fn extract_content_from_response_returns_empty_for_non_text_responses() {
    assert_eq!(
        extract_content_from_response(&CallResponse::Other).unwrap(),
        ""
    );
}

#[test]
fn model_response_rejects_a_choice_of_another_shape() {
    let parsed = serde_json::from_str::<ModelResponse>(r#"{"choices":["text"]}"#);
    parsed.unwrap_err();
}
