#![cfg(test)]

use std::slice;
use std::time::Duration;

use rstest::rstest;
use secrecy::SecretString;

use super::super::counter::{CountTokensCall, TokenCounter as _};
use super::super::error::CountTokensError;
use super::super::handler::HandlerRequest;
use super::super::messages::{
    ChatMessage, ContentRole, ResponsesContent, blocks_to_content, messages_to_responses_input,
};
use super::super::python_text::{PyValue, display_text};
use super::super::request::{ResponsesInput, ResponsesTool, flatten_tools};
use super::{KEY, OK, counter, params, run};

#[rstest]
#[case::empty_messages(r#"{"model":"m","key":"k","messages":[]}"#)]
#[case::no_key_anywhere(
    r#"{"model":"m","no_key":true,"messages":[{"role":"user","content":"hi"}]}"#
)]
#[tokio::test]
async fn returns_none_before_any_http(#[case] scenario: &str) {
    let (rendered, request) = run(scenario, 200, OK).await;
    assert_eq!(rendered, "none");
    assert!(request.is_none());
}

#[rstest]
#[case::text(
    r#"{"model":"m","key":"k","system":"plain","messages":[{"role":"user","content":"hi"}]}"#,
    "plain"
)]
#[case::object(r#"{"model":"m","key":"k","system":{"b":1,"a":[null,false]},"messages":[{"role":"user","content":"hi"}]}"#, "{'b': 1, 'a': [None, False]}")]
#[tokio::test]
async fn stringifies_the_system_fallback(#[case] scenario: &str, #[case] instructions: &str) {
    let (_, request) = run(scenario, 200, OK).await;
    assert_eq!(request.unwrap().1["instructions"], instructions);
}

#[rstest]
#[case::openai(Some("openai"), true)]
#[case::azure(Some("azure"), false)]
#[case::absent(None, false)]
fn only_openai_selects_the_native_counter(#[case] provider: Option<&str>, #[case] native: bool) {
    assert_eq!(
        counter(None).should_use_token_counting_api(provider),
        native
    );
}

#[rstest]
#[case::null_tokens(r#"{"input_tokens":null}"#)]
#[case::text_tokens(r#"{"input_tokens":"7"}"#)]
#[case::float_tokens(r#"{"input_tokens":1.5}"#)]
#[tokio::test]
async fn a_non_integer_input_tokens_is_an_error_record(#[case] answer: &str) {
    let scenario = r#"{"model":"m","key":"k","messages":[{"role":"user","content":"hi"}]}"#;
    let (rendered, _) = run(scenario, 200, answer).await;
    assert_eq!(rendered, "error 500: input_tokens is not an integer");
}

#[tokio::test]
async fn the_handler_wraps_validation_and_transport_failures_as_500() {
    let handler = counter(None).handler;
    let key = SecretString::from(KEY.to_owned());
    let items = ResponsesInput::Items(Vec::new());
    let request = HandlerRequest {
        model: "m",
        input: &items,
        api_key: &key,
        api_base: None,
        timeout: None,
        tools: None,
        instructions: None,
    };
    let invalid = handler
        .handle_count_tokens_request(&request)
        .await
        .unwrap_err();
    assert!(
        matches!(&invalid, CountTokensError::Api(api) if api.status_code == 500
        && api.message == "CountTokens processing error: input parameter is required")
    );
    let text = ResponsesInput::Text(String::from("hi"));
    let unreachable = HandlerRequest {
        api_base: Some("http://127.0.0.1:1/v1"),
        input: &text,
        ..request
    };
    let error = handler
        .handle_count_tokens_request(&unreachable)
        .await
        .unwrap_err();
    assert!(
        matches!(&error, CountTokensError::Api(api) if api.status_code == 500
        && api.message.starts_with("CountTokens processing error: "))
    );
}

#[rstest]
#[case::default_base_and_injected_timeout(
    None,
    None,
    "https://api.openai.com/v1/responses/input_tokens",
    6000
)]
#[case::trimmed_base_and_explicit_timeout(
    Some("http://h/v1///"),
    Some(3),
    "http://h/v1/responses/input_tokens",
    3
)]
#[case::empty_base_is_the_default(
    Some(""),
    None,
    "https://api.openai.com/v1/responses/input_tokens",
    6000
)]
fn prepares_the_request_url_headers_and_deadline(
    #[case] base: Option<&str>,
    #[case] timeout: Option<u64>,
    #[case] url: &str,
    #[case] seconds: u64,
) {
    let handler = counter(None).handler;
    let key = SecretString::from(KEY.to_owned());
    let input = ResponsesInput::Text(String::from("hi"));
    let request = HandlerRequest {
        model: "m",
        input: &input,
        api_key: &key,
        api_base: base,
        timeout: timeout.map(Duration::from_secs),
        tools: None,
        instructions: None,
    };
    let outbound = handler.prepare(&request).unwrap();
    assert_eq!(outbound.url.as_str(), url);
    assert_eq!(outbound.deadline, Duration::from_secs(seconds));
    assert_eq!(outbound.headers["content-type"], "application/json");
    assert_eq!(
        outbound.headers["authorization"],
        format!("Bearer {KEY}").as_str()
    );
    assert!(outbound.headers["authorization"].is_sensitive());
}

#[rstest]
#[case::both_quotes(r#"["it's \"x\""]"#, r#"['it\'s "x"']"#)]
#[case::low_control(r#"["a\u0007b"]"#, r"['a\x07b']")]
#[case::high_control(r#"["a\u0085b"]"#, r"['a\x85b']")]
fn prints_awkward_strings_like_python(#[case] value: &str, #[case] printed: &str) {
    let parsed: PyValue = serde_json::from_str(value).unwrap();
    assert_eq!(display_text(&parsed), printed);
}

#[test]
fn a_tool_with_a_function_key_but_another_type_passes_through() {
    let tool: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(r#"{"type":"custom","function":{"name":"x"}}"#).unwrap();
    let flattened = flatten_tools(slice::from_ref(&tool)).unwrap();
    assert_eq!(flattened, vec![ResponsesTool::Other(tool)]);
}

#[tokio::test]
async fn null_and_non_string_function_fields_are_sent_as_given() {
    let scenario = r#"{"model":"m","key":"k",
        "tools":[{"type":"function","function":{"name":123,"description":null,"parameters":null,"strict":null}}],
        "messages":[
            {"role":"assistant","content":"","tool_calls":[{"id":null,"function":{"name":5,"arguments":null}}]},
            {"role":"tool","tool_call_id":null,"content":"done"}]}"#;
    let (_, request) = run(scenario, 200, OK).await;
    let expected: serde_json::Value = serde_json::from_str(
        r#"{"model":"m",
        "tools":[{"type":"function","name":123,"description":null,"parameters":null,"strict":null}],
        "input":[{"type":"function_call","call_id":null,"name":5,"arguments":null},
            {"type":"function_call_output","call_id":null,"output":"done"}]}"#,
    )
    .unwrap();
    assert_eq!(request.unwrap().1, expected);
}

#[test]
fn an_empty_image_url_string_is_dropped() {
    let blocks: Vec<PyValue> =
        serde_json::from_str(r#"[{"type":"image_url","image_url":""}]"#).unwrap();
    assert_eq!(
        blocks_to_content(&blocks, ContentRole::User),
        ResponsesContent::Text(String::new())
    );
}

#[test]
fn an_assistant_with_tool_calls_and_no_text_sends_only_the_calls() {
    let message: ChatMessage = serde_json::from_str(
        r#"{"role":"assistant","content":"","tool_calls":[{"id":"c","function":{"name":"f","arguments":"{}"}}]}"#,
    )
    .unwrap();
    let (items, _) = messages_to_responses_input(&[message]);
    assert_eq!(
        serde_json::to_value(items).unwrap(),
        serde_json::from_str::<serde_json::Value>(
            r#"[{"type":"function_call","call_id":"c","name":"f","arguments":"{}"}]"#
        )
        .unwrap()
    );
}

#[test]
fn an_assistant_media_only_message_becomes_an_empty_message() {
    let message: ChatMessage = serde_json::from_str(
        r#"{"role":"assistant","content":[{"type":"image_url","image_url":"http://i/1.png"}]}"#,
    )
    .unwrap();
    let (items, _) = messages_to_responses_input(&[message]);
    assert_eq!(
        serde_json::to_value(items).unwrap(),
        serde_json::from_str::<serde_json::Value>(r#"[{"role":"assistant","content":""}]"#)
            .unwrap()
    );
}

#[rstest]
#[case::malformed_json("not json", "the answer is not JSON: ")]
#[case::not_an_object("[1]", "the answer is not a JSON object")]
#[tokio::test]
async fn an_unusable_200_answer_is_a_500_error_record(#[case] answer: &str, #[case] reason: &str) {
    let scenario = r#"{"model":"m","key":"k","messages":[{"role":"user","content":"hi"}]}"#;
    let (rendered, _) = run(scenario, 200, answer).await;
    assert!(
        rendered.starts_with(&format!(
            "error 500: CountTokens processing error: {reason}"
        )),
        "{rendered}"
    );
}

#[tokio::test]
async fn a_null_200_answer_returns_none() {
    let scenario = r#"{"model":"m","key":"k","messages":[{"role":"user","content":"hi"}]}"#;
    let (rendered, request) = run(scenario, 200, "null").await;
    assert_eq!(rendered, "none");
    assert!(request.is_some());
}

#[tokio::test]
async fn a_transport_failure_is_a_500_error_record() {
    let message: ChatMessage = serde_json::from_str(r#"{"role":"user","content":"hi"}"#).unwrap();
    let deployment = params(
        Some(String::from(KEY)),
        String::from("http://127.0.0.1:1/v1"),
    );
    let call = CountTokensCall {
        model_to_use: "m",
        messages: slice::from_ref(&message),
        deployment: Some(&deployment),
        request_model: "requested",
        tools: None,
        system: None,
    };
    let response = counter(None).count_tokens(&call).await.unwrap();
    assert!(response.error);
    assert_eq!(response.status_code, Some(500));
    assert!(
        response
            .error_message
            .as_deref()
            .is_some_and(|text| text.starts_with("CountTokens processing error: "))
    );
}
