use betterlite_prompts::arguments::json::Json;
use rstest::rstest;

use super::super::config::OllamaChatConfig;
use super::super::request::{OllamaChatRequest, RequestInput, transform_request};
use super::support::{messages, options};

fn expected(json: &str) -> Json {
    serde_json::from_str(json).unwrap()
}

fn transformed(
    model: &str,
    message_json: &str,
    option_json: &str,
) -> Result<OllamaChatRequest, super::super::error::OllamaError> {
    let parsed_messages = messages(message_json);
    let optional = options(option_json);
    let input = RequestInput {
        model,
        messages: &parsed_messages,
        optional_params: &optional,
    };
    transform_request(&input, &OllamaChatConfig::default())
}

fn body(request: &OllamaChatRequest) -> Json {
    serde_json::from_str(&serde_json::to_string(request).unwrap()).unwrap()
}

#[rstest]
#[case::plain(
    r#"[{"role": "user", "content": "hi"}]"#,
    "{}",
    r#"{"model": "llama3", "messages": [{"role": "user", "content": "hi", "images": []}], "options": {}, "stream": false}"#
)]
#[case::content_parts(
    r#"[{"role": "system", "content": [{"type": "text", "text": "a"}, {"type": "text", "text": "b"}]}]"#,
    "{}",
    r#"{"model": "llama3", "messages": [{"role": "system", "content": "ab", "images": []}], "options": {}, "stream": false}"#
)]
#[case::null_content(
    r#"[{"role": "user", "content": null}]"#,
    "{}",
    r#"{"model": "llama3", "messages": [{"role": "user", "content": "", "images": []}], "options": {}, "stream": false}"#
)]
#[case::empty_content(
    r#"[{"role": "user", "content": ""}]"#,
    "{}",
    r#"{"model": "llama3", "messages": [{"role": "user", "content": "", "images": []}], "options": {}, "stream": false}"#
)]
#[case::search_results(
    r#"[{"role": "user", "content": "x", "search_results": [{"source": "s", "title": "t", "content": [{"text": "c"}], "citations": {"x": 1}}]}]"#,
    "{}",
    r#"{"model": "llama3", "messages": [{"role": "user", "content": "xstc{\"x\":1}", "images": []}], "options": {}, "stream": false}"#
)]
#[case::reasoning_content(
    r#"[{"role": "assistant", "content": "hello", "reasoning_content": "why"}]"#,
    "{}",
    r#"{"model": "llama3", "messages": [{"role": "assistant", "thinking": "why", "content": "hello", "images": []}], "options": {}, "stream": false}"#
)]
#[case::reasoning_alias(
    r#"[{"role": "assistant", "content": "hello", "reasoning": "why"}]"#,
    "{}",
    r#"{"model": "llama3", "messages": [{"role": "assistant", "thinking": "why", "content": "hello", "images": []}], "options": {}, "stream": false}"#
)]
#[case::reasoning_content_wins(
    r#"[{"role": "assistant", "content": "hello", "reasoning_content": "one", "reasoning": "two"}]"#,
    "{}",
    r#"{"model": "llama3", "messages": [{"role": "assistant", "thinking": "one", "content": "hello", "images": []}], "options": {}, "stream": false}"#
)]
#[case::empty_reasoning_is_kept(
    r#"[{"role": "assistant", "content": "hello", "reasoning_content": ""}]"#,
    "{}",
    r#"{"model": "llama3", "messages": [{"role": "assistant", "thinking": "", "content": "hello", "images": []}], "options": {}, "stream": false}"#
)]
#[case::null_reasoning_is_dropped(
    r#"[{"role": "assistant", "content": "hello", "reasoning_content": null}]"#,
    "{}",
    r#"{"model": "llama3", "messages": [{"role": "assistant", "content": "hello", "images": []}], "options": {}, "stream": false}"#
)]
#[case::null_reasoning_stops_the_tag_parse(
    r#"[{"role": "assistant", "content": "<think>inner</think>answer", "reasoning": null}]"#,
    "{}",
    r#"{"model": "llama3", "messages": [{"role": "assistant", "content": "<think>inner</think>answer", "images": []}], "options": {}, "stream": false}"#
)]
#[case::think_tags_stay_in_content(
    r#"[{"role": "assistant", "content": "<think>inner</think>answer"}]"#,
    "{}",
    r#"{"model": "llama3", "messages": [{"role": "assistant", "thinking": "inner", "content": "<think>inner</think>answer", "images": []}], "options": {}, "stream": false}"#
)]
#[case::tool_call_id(
    r#"[{"role": "tool", "content": "r", "tool_call_id": "call_1"}]"#,
    "{}",
    r#"{"model": "llama3", "messages": [{"role": "tool", "content": "r", "images": [], "tool_call_id": "call_1"}], "options": {}, "stream": false}"#
)]
#[case::null_tool_call_id_is_dropped(
    r#"[{"role": "tool", "content": "r", "tool_call_id": null}]"#,
    "{}",
    r#"{"model": "llama3", "messages": [{"role": "tool", "content": "r", "images": []}], "options": {}, "stream": false}"#
)]
#[case::stream_and_top_level_options(
    r#"[{"role": "user", "content": "hi"}]"#,
    r#"{"stream": true, "temperature": 0.5, "format": {"type": "object"}, "tools": [{"type": "function"}], "keep_alive": "5m", "think": true, "function_name": "f"}"#,
    r#"{"model": "llama3", "messages": [{"role": "user", "content": "hi", "images": []}], "options": {"temperature": 0.5}, "stream": true, "format": {"type": "object"}, "tools": [{"type": "function"}], "keep_alive": "5m", "think": true}"#
)]
#[case::null_options_are_dropped(
    r#"[{"role": "user", "content": "hi"}]"#,
    r#"{"format": null, "tools": null, "keep_alive": null, "think": null, "function_name": null}"#,
    r#"{"model": "llama3", "messages": [{"role": "user", "content": "hi", "images": []}], "options": {}, "stream": false}"#
)]
#[case::false_think_is_kept(
    r#"[{"role": "user", "content": "hi"}]"#,
    r#"{"think": false, "tools": []}"#,
    r#"{"model": "llama3", "messages": [{"role": "user", "content": "hi", "images": []}], "options": {}, "stream": false, "tools": [], "think": false}"#
)]
#[case::tool_calls(
    r#"[{"role": "assistant", "content": "x", "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "f", "arguments": "{\"a\": 1}"}}, {"id": "c2", "type": "other", "function": {"name": "g", "arguments": "{}"}}]}]"#,
    "{}",
    r#"{"model": "llama3", "messages": [{"role": "assistant", "content": "x", "images": [], "tool_calls": [{"function": {"name": "f", "arguments": {"a": 1}}}]}], "options": {}, "stream": false}"#
)]
#[case::tool_call_without_arguments(
    r#"[{"role": "assistant", "content": "x", "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "f"}}]}]"#,
    "{}",
    r#"{"model": "llama3", "messages": [{"role": "assistant", "content": "x", "images": [], "tool_calls": [{"function": {"name": "f", "arguments": {}}}]}], "options": {}, "stream": false}"#
)]
#[case::empty_tool_calls(
    r#"[{"role": "assistant", "content": "x", "tool_calls": []}]"#,
    "{}",
    r#"{"model": "llama3", "messages": [{"role": "assistant", "content": "x", "images": [], "tool_calls": []}], "options": {}, "stream": false}"#
)]
#[case::null_tool_calls(
    r#"[{"role": "assistant", "content": "x", "tool_calls": null}]"#,
    "{}",
    r#"{"model": "llama3", "messages": [{"role": "assistant", "content": "x", "images": []}], "options": {}, "stream": false}"#
)]
fn transform_request_bodies_match_litellm(
    #[case] message_json: &str,
    #[case] option_json: &str,
    #[case] expected_body: &str,
) {
    let request = transformed("llama3", message_json, option_json).unwrap();
    assert_eq!(body(&request), expected(expected_body));
}

#[rstest]
#[case::data_url_string(
    r#"[{"role": "user", "content": [{"type": "text", "text": "look"}, {"type": "image_url", "image_url": "data:image/png;base64,AA"}]}]"#,
    "AA"
)]
#[case::data_url_object(
    r#"[{"role": "user", "content": [{"type": "image_url", "image_url": {"url": "data:image/jpeg;base64,BB"}}]}]"#,
    "BB"
)]
fn images_are_extracted_as_base64(#[case] message_json: &str, #[case] expected: &str) {
    let request = transformed("llama3", message_json, "{}").unwrap();
    assert_eq!(request.messages.first().unwrap().images, [expected]);
}

#[test]
fn a_plain_image_url_is_kept() {
    let request = transformed(
        "llama3",
        r#"[{"role": "user", "content": [{"type": "image_url", "image_url": {"url": "http://h/i.png"}}]}]"#,
        "{}",
    )
    .unwrap();
    assert_eq!(request.messages.first().unwrap().images, ["http://h/i.png"]);
}

#[test]
fn a_message_without_images_still_carries_an_empty_list() {
    let request = transformed("llama3", r#"[{"role": "user", "content": "hi"}]"#, "{}").unwrap();
    assert!(request.messages.first().unwrap().images.is_empty());
}

#[test]
fn the_function_name_is_returned_for_the_reply_transform() {
    let request = transformed(
        "llama3",
        r#"[{"role": "user", "content": "hi"}]"#,
        r#"{"function_name": "f"}"#,
    )
    .unwrap();
    assert_eq!(request.function_name, Some(Json::Text(String::from("f"))));
}

#[test]
fn config_options_fill_in_what_the_request_does_not_set() {
    let config: OllamaChatConfig =
        serde_json::from_str(r#"{"mirostat": 1, "temperature": 0.5, "seed": 2}"#).unwrap();
    let message_json = r#"[{"role": "user", "content": "hi"}]"#;
    let parsed_messages = messages(message_json);
    let optional = options(r#"{"temperature": 0.9}"#);
    let input = RequestInput {
        model: "llama3",
        messages: &parsed_messages,
        optional_params: &optional,
    };
    let request = transform_request(&input, &config).unwrap();
    assert_eq!(
        request.options,
        options(r#"{"temperature": 0.9, "mirostat": 1, "seed": 2}"#)
    );
}

#[test]
fn tool_call_arguments_that_are_not_json_are_a_client_error() {
    let error = transformed(
        "llama3",
        r#"[{"role": "assistant", "content": "x", "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "f", "arguments": "not json"}}]}]"#,
        "{}",
    )
    .unwrap_err();
    assert_eq!(error.status(), Some(400));
}

#[test]
fn a_stream_option_that_is_not_a_bool_is_a_client_error() {
    let error = transformed(
        "llama3",
        r#"[{"role": "user", "content": "hi"}]"#,
        r#"{"stream": "yes"}"#,
    )
    .unwrap_err();
    assert_eq!(error.status(), Some(400));
}

#[test]
fn a_tool_call_without_a_name_gets_an_empty_one() {
    let request = transformed(
        "llama3",
        r#"[{"role": "assistant", "content": "x", "tool_calls": [{"id": "c1", "type": "function", "function": {"arguments": "{}"}}]}]"#,
        "{}",
    )
    .unwrap();
    assert_eq!(
        body(&request),
        expected(
            r#"{"model": "llama3", "messages": [{"role": "assistant", "content": "x", "images": [], "tool_calls": [{"function": {"name": "", "arguments": {}}}]}], "options": {}, "stream": false}"#
        )
    );
}

#[test]
fn a_wrong_typed_reasoning_field_is_a_client_error() {
    let error = transformed(
        "llama3",
        r#"[{"role": "assistant", "content": "x", "reasoning_content": 5}]"#,
        "{}",
    )
    .unwrap_err();
    assert_eq!(error.status(), Some(400));
}

#[test]
fn a_wrong_typed_tool_call_id_is_a_client_error() {
    let error = transformed(
        "llama3",
        r#"[{"role": "tool", "content": "r", "tool_call_id": 5}]"#,
        "{}",
    )
    .unwrap_err();
    assert_eq!(error.status(), Some(400));
}
