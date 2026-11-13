#![cfg(test)]

use indexmap::IndexMap;
use rstest::rstest;
use serde_json::{Value, json};

use super::{supported_openai_params, transform_messages};
use crate::chat::ChatMessage;
use crate::hosted_chat_options::map_renaming_token_limit;

fn message(text: &str) -> ChatMessage {
    serde_json::from_str(text).unwrap()
}

fn requested(text: &str) -> IndexMap<String, Value> {
    serde_json::from_str(text).unwrap()
}

fn mapped(
    gate: bool,
    requested: &IndexMap<String, Value>,
    existing: &IndexMap<String, Value>,
) -> IndexMap<String, Value> {
    let mut merged = existing.clone();
    map_renaming_token_limit(requested, &mut merged, &supported_openai_params(gate));
    merged
}

#[test]
fn supported_params_are_the_nine_chat_params_in_order() {
    assert_eq!(
        supported_openai_params(false),
        [
            "max_completion_tokens",
            "max_tokens",
            "response_format",
            "stop",
            "stream",
            "stream_options",
            "temperature",
            "top_p",
            "top_k",
        ]
    );
}

#[test]
fn function_calling_appends_tools_tool_choice_and_parallel_tool_calls() {
    assert_eq!(
        supported_openai_params(true),
        [
            "max_completion_tokens",
            "max_tokens",
            "response_format",
            "stop",
            "stream",
            "stream_options",
            "temperature",
            "top_p",
            "top_k",
            "tools",
            "tool_choice",
            "parallel_tool_calls",
        ]
    );
}

#[test]
fn max_completion_tokens_is_renamed_and_an_existing_max_tokens_survives() {
    let merged = mapped(
        false,
        &requested(r#"{"max_completion_tokens": 20}"#),
        &requested(r#"{"max_tokens": 10}"#),
    );
    assert_eq!(merged["max_tokens"], json!(20));
    assert_eq!(merged.len(), 1);
}

#[rstest]
#[case::temperature(r#"{"temperature": 0.2}"#)]
#[case::top_k(r#"{"top_k": 40}"#)]
#[case::stream_options(r#"{"stream_options": {"include_usage": true}}"#)]
fn a_supported_param_is_copied_as_given(#[case] text: &str) {
    let merged = mapped(false, &requested(text), &IndexMap::new());
    assert_eq!(merged, requested(text));
}

#[rstest]
#[case::tools(r#"{"tools": [{"type": "function"}]}"#, true)]
#[case::tool_choice(r#"{"tool_choice": "auto"}"#, true)]
#[case::parallel_tool_calls(r#"{"parallel_tool_calls": false}"#, true)]
fn the_tools_params_cross_only_when_function_calling_is_supported(
    #[case] text: &str,
    #[case] gate: bool,
) {
    let merged = mapped(gate, &requested(text), &IndexMap::new());
    assert_eq!(merged, requested(text));
}

#[rstest]
#[case::tools(r#"{"tools": [{"type": "function"}]}"#)]
#[case::tool_choice(r#"{"tool_choice": "auto"}"#)]
#[case::parallel_tool_calls(r#"{"parallel_tool_calls": false}"#)]
fn the_tools_params_are_dropped_without_function_calling(#[case] text: &str) {
    let merged = mapped(false, &requested(text), &IndexMap::new());
    assert!(merged.is_empty());
}

#[rstest]
#[case::unsupported(r#"{"logprobs": true}"#)]
#[case::unknown(r#"{"seed": 4}"#)]
fn an_unsupported_param_is_dropped_without_an_error(#[case] text: &str) {
    let merged = mapped(true, &requested(text), &IndexMap::new());
    assert!(merged.is_empty());
}

#[test]
fn a_text_part_list_becomes_one_concatenated_string() {
    let converted = transform_messages(&[message(
        r#"{"role": "user", "content": [{"type": "text", "text": "one "}, {"type": "text", "text": "two"}]}"#,
    )])
    .unwrap();
    assert_eq!(
        converted[0].content,
        Some(crate::chat::MessageContent::Text("one two".to_owned()))
    );
}

#[test]
fn a_list_without_text_parts_stays_a_list() {
    let converted = transform_messages(&[message(
        r#"{"role": "user", "content": [{"type": "image_url", "image_url": {"url": "http://x.test/a.png"}}]}"#,
    )])
    .unwrap();
    assert!(matches!(
        &converted[0].content,
        Some(crate::chat::MessageContent::Parts(parts)) if parts.len() == 1
    ));
}

#[test]
fn a_string_content_is_left_alone() {
    let converted =
        transform_messages(&[message(r#"{"role": "user", "content": "plain"}"#)]).unwrap();
    assert_eq!(
        converted[0].content,
        Some(crate::chat::MessageContent::Text("plain".to_owned()))
    );
}

#[test]
fn search_results_text_is_appended_to_the_flattened_content() {
    let converted = transform_messages(&[message(
        r#"{"role": "user", "content": "weather", "search_results": [{"title": "sunny"}]}"#,
    )])
    .unwrap();
    assert_eq!(
        converted[0].content,
        Some(crate::chat::MessageContent::Text("weathersunny".to_owned()))
    );
    assert_eq!(
        converted[0].passthrough["search_results"][0]["title"],
        json!("sunny")
    );
}

#[test]
fn tool_calls_survive_the_transform() {
    let converted = transform_messages(&[message(
        r#"{"role": "assistant", "content": [{"type": "text", "text": "hi"}], "tool_calls": [{"id": "a", "type": "function", "function": {"name": "f", "arguments": "{}"}}]}"#,
    )])
    .unwrap();
    assert_eq!(converted[0].tool_calls.as_ref().map(Vec::len), Some(1));
}

#[test]
fn a_message_without_content_is_unchanged() {
    let converted = transform_messages(&[message(r#"{"role": "assistant"}"#)]).unwrap();
    assert_eq!(converted[0].content, None);
}

#[test]
fn every_message_in_the_list_is_converted() {
    let converted = transform_messages(&[
        message(r#"{"role": "user", "content": [{"type": "text", "text": "a"}]}"#),
        message(r#"{"role": "assistant", "content": "b"}"#),
    ])
    .unwrap();
    assert_eq!(converted.len(), 2);
    assert_eq!(
        converted[0].content,
        Some(crate::chat::MessageContent::Text("a".to_owned()))
    );
}
