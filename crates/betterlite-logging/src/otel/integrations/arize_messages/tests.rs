#![cfg(test)]

use std::collections::BTreeMap;

use super::text::coerce_text;
use super::tool_calls::{normalize_tool_call, summarize_tool_calls};
use super::{ArizeOtelAttributes, set_response_attributes};
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::span_attributes::LLMObsAttributes as _;
use opentelemetry::Value;
use opentelemetry::trace::{Span as _, Tracer as _, TracerProvider as _};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, Span as SdkSpan};

fn json(raw: &str) -> OtelModelValue {
    serde_json::from_str(raw).expect("json value")
}

fn recorded<F>(record: F) -> BTreeMap<String, Value>
where
    F: FnOnce(&mut SdkSpan),
{
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let mut span = provider.tracer("arize-messages").start("test");
    record(&mut span);
    span.end();
    let spans = exporter.get_finished_spans().expect("spans");
    spans
        .first()
        .expect("one span")
        .attributes
        .iter()
        .map(|pair| (pair.key.to_string(), pair.value.clone()))
        .collect()
}

fn expected(pairs: &[(&str, Value)]) -> BTreeMap<String, Value> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn message_attributes(messages: &str) -> BTreeMap<String, Value> {
    let messages = json(messages);
    recorded(|span| {
        let mut kwargs = MetadataMap::new();
        kwargs.insert("messages".to_owned(), messages.clone());
        ArizeOtelAttributes::set_messages(span, &kwargs);
    })
}

fn response_attributes(response: &str) -> BTreeMap<String, Value> {
    let response = json(response);
    recorded(|span| set_response_attributes(span, &response))
}

#[test]
fn input_messages_write_last_value_and_each_role_and_content() {
    let attributes = message_attributes(
        r#"[{"role":"system","content":"be brief"},{"role":"user","content":"hello"}]"#,
    );
    assert_eq!(
        attributes,
        expected(&[
            ("input.value", Value::from("hello")),
            ("llm.input_messages.0.message.role", Value::from("system")),
            (
                "llm.input_messages.0.message.content",
                Value::from("be brief")
            ),
            ("llm.input_messages.1.message.role", Value::from("user")),
            ("llm.input_messages.1.message.content", Value::from("hello")),
        ])
    );
}

#[test]
fn input_messages_empty_list_writes_nothing() {
    assert!(message_attributes("[]").is_empty());
}

#[test]
fn input_messages_null_content_writes_empty_strings() {
    let attributes = message_attributes(r#"[{"role":"user","content":null}]"#);
    assert_eq!(attributes.get("input.value"), Some(&Value::from("")));
    assert_eq!(
        attributes.get("llm.input_messages.0.message.content"),
        Some(&Value::from(""))
    );
    assert_eq!(
        attributes.get("llm.input_messages.0.message.role"),
        Some(&Value::from("user"))
    );
}

#[test]
fn input_messages_name_and_tool_call_id_are_written() {
    let attributes = message_attributes(
        r#"[{"role":"tool","content":"sunny","name":"get_weather","tool_call_id":"call_1"}]"#,
    );
    assert_eq!(
        attributes.get("llm.input_messages.0.message.name"),
        Some(&Value::from("get_weather"))
    );
    assert_eq!(
        attributes.get("llm.input_messages.0.message.tool_call_id"),
        Some(&Value::from("call_1"))
    );
}

#[test]
fn input_messages_skip_non_object_entries() {
    let attributes = message_attributes(r#"[{"role":"user","content":"hi"},7]"#);
    assert_eq!(attributes.get("input.value"), Some(&Value::from("")));
    assert_eq!(
        attributes.get("llm.input_messages.0.message.content"),
        Some(&Value::from("hi"))
    );
    assert!(!attributes.contains_key("llm.input_messages.1.message.role"));
}

#[test]
fn input_messages_array_content_is_written_as_json_text() {
    let attributes =
        message_attributes(r#"[{"role":"user","content":[{"type":"text","text":"hello"}]}]"#);
    let text = Value::from(r#"[{"text":"hello","type":"text"}]"#);
    assert_eq!(attributes.get("input.value"), Some(&text));
    assert_eq!(
        attributes.get("llm.input_messages.0.message.content"),
        Some(&text)
    );
}

#[test]
fn multimodal_text_and_image_url_parts() {
    let attributes = message_attributes(
        r#"[{"role":"user","content":[{"type":"text","text":"Describe"},{"type":"image_url","image_url":{"url":"https://example.com/cat.png"}}]}]"#,
    );
    assert_eq!(
        attributes.get("llm.input_messages.0.message.contents.0.message_content.type"),
        Some(&Value::from("text"))
    );
    assert_eq!(
        attributes.get("llm.input_messages.0.message.contents.0.message_content.text"),
        Some(&Value::from("Describe"))
    );
    assert_eq!(
        attributes.get("llm.input_messages.0.message.contents.1.message_content.type"),
        Some(&Value::from("image"))
    );
    assert_eq!(
        attributes.get("llm.input_messages.0.message.contents.1.message_content.image.image.url"),
        Some(&Value::from("https://example.com/cat.png"))
    );
}

#[test]
fn multimodal_part_types_image_and_input_image() {
    let attributes = message_attributes(
        r#"[{"role":"user","content":[{"type":"image","image_url":{"url":"https://example.com/a.png"}},{"type":"input_image","image_url":"https://example.com/b.png"}]}]"#,
    );
    assert_eq!(
        attributes.get("llm.input_messages.0.message.contents.0.message_content.image.image.url"),
        Some(&Value::from("https://example.com/a.png"))
    );
    assert_eq!(
        attributes.get("llm.input_messages.0.message.contents.1.message_content.image.image.url"),
        Some(&Value::from("https://example.com/b.png"))
    );
}

#[test]
fn multimodal_image_url_string_is_the_url() {
    let attributes = message_attributes(
        r#"[{"role":"user","content":[{"type":"image_url","image_url":"https://example.com/c.png"}]}]"#,
    );
    assert_eq!(
        attributes.get("llm.input_messages.0.message.contents.0.message_content.image.image.url"),
        Some(&Value::from("https://example.com/c.png"))
    );
}

#[test]
fn multimodal_anthropic_source_becomes_a_data_url() {
    let attributes = message_attributes(
        r#"[{"role":"user","content":[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"QUJD"}},{"type":"image","source":{"type":"base64","data":"REVG"}}]}]"#,
    );
    assert_eq!(
        attributes.get("llm.input_messages.0.message.contents.0.message_content.image.image.url"),
        Some(&Value::from("data:image/png;base64,QUJD"))
    );
    assert_eq!(
        attributes.get("llm.input_messages.0.message.contents.1.message_content.image.image.url"),
        Some(&Value::from("data:image/jpeg;base64,REVG"))
    );
}

#[test]
fn multimodal_source_without_data_uses_part_url() {
    let attributes = message_attributes(
        r#"[{"role":"user","content":[{"type":"image","source":{"type":"base64","data":""},"url":"https://example.com/d.png"}]}]"#,
    );
    assert_eq!(
        attributes.get("llm.input_messages.0.message.contents.0.message_content.image.image.url"),
        Some(&Value::from("https://example.com/d.png"))
    );
}

#[test]
fn multimodal_non_object_part_keeps_index() {
    let attributes = message_attributes(
        r#"[{"role":"user","content":["plain text",{"type":"text","text":"hi"}]}]"#,
    );
    assert!(
        !attributes.contains_key("llm.input_messages.0.message.contents.0.message_content.type")
    );
    assert_eq!(
        attributes.get("llm.input_messages.0.message.contents.1.message_content.text"),
        Some(&Value::from("hi"))
    );
}

#[test]
fn tool_calls_normalize_object_arguments_and_default_type() {
    let call = normalize_tool_call(&json(
        r#"{"id":"call_1","function":{"name":"lookup","arguments":{"q":"x"}}}"#,
    ))
    .expect("normalized call");
    assert_eq!(
        serde_json::to_string(&call).expect("json"),
        r#"{"id":"call_1","type":"function","function":{"name":"lookup","arguments":"{\"q\":\"x\"}"}}"#
    );
    assert!(normalize_tool_call(&json("7")).is_none());
}

#[test]
fn tool_calls_summary_skips_non_objects() {
    let calls = [
        json("7"),
        json(
            r#"{"id":"call_1","type":"function","function":{"name":"lookup","arguments":"{\"q\":\"x\"}"}}"#,
        ),
        json("null"),
    ];
    assert_eq!(
        summarize_tool_calls(&calls),
        r#"{"tool_calls":[{"id":"call_1","type":"function","function":{"name":"lookup","arguments":"{\"q\":\"x\"}"}}]}"#
    );
    assert_eq!(
        summarize_tool_calls(&[json(r#"{"id":"call_2"}"#)]),
        r#"{"tool_calls":[{"id":"call_2","type":"function","function":{"name":null,"arguments":null}}]}"#
    );
}

#[test]
fn tool_calls_emit_skips_empty_id_and_name_but_writes_empty_arguments() {
    let attributes = message_attributes(
        r#"[{"role":"assistant","content":null,"tool_calls":[{"id":"","type":"function","function":{"name":"","arguments":""}},{"id":"call_2","function":{"name":"lookup","arguments":"{}"}}]}]"#,
    );
    let first = "llm.input_messages.0.message.tool_calls.0.tool_call";
    assert!(!attributes.contains_key(&format!("{first}.id")));
    assert!(!attributes.contains_key(&format!("{first}.function.name")));
    assert_eq!(
        attributes.get(&format!("{first}.function.arguments")),
        Some(&Value::from(""))
    );
    let second = "llm.input_messages.0.message.tool_calls.1.tool_call";
    assert_eq!(
        attributes.get(&format!("{second}.id")),
        Some(&Value::from("call_2"))
    );
    assert_eq!(
        attributes.get(&format!("{second}.function.name")),
        Some(&Value::from("lookup"))
    );
    assert_eq!(
        attributes.get(&format!("{second}.function.arguments")),
        Some(&Value::from("{}"))
    );
}

#[test]
fn choices_text_message_writes_output_value_role_and_content() {
    let attributes = response_attributes(
        r#"{"choices":[{"message":{"role":"assistant","content":"Hello there"}}]}"#,
    );
    assert_eq!(
        attributes,
        expected(&[
            ("output.value", Value::from("Hello there")),
            (
                "llm.output_messages.0.message.role",
                Value::from("assistant")
            ),
            (
                "llm.output_messages.0.message.content",
                Value::from("Hello there")
            ),
        ])
    );
}

#[test]
fn choices_tool_only_message_writes_summary_as_output_value() {
    let attributes = response_attributes(
        r#"{"choices":[{"message":{"role":"assistant","content":"","tool_calls":[{"id":"call_1","type":"function","function":{"name":"lookup","arguments":"{}"}}]}}]}"#,
    );
    assert_eq!(
        attributes.get("output.value"),
        Some(&Value::from(
            r#"{"tool_calls":[{"id":"call_1","type":"function","function":{"name":"lookup","arguments":"{}"}}]}"#
        ))
    );
    assert_eq!(
        attributes.get("llm.output_messages.0.message.content"),
        Some(&Value::from(""))
    );
    assert_eq!(
        attributes.get("llm.output_messages.0.message.tool_calls.0.tool_call.id"),
        Some(&Value::from("call_1"))
    );
}

#[test]
fn choices_last_choice_keeps_output_value() {
    let attributes = response_attributes(
        r#"{"choices":[{"message":{"role":"assistant","content":"one"}},{"message":{"role":"assistant","content":"two"}}]}"#,
    );
    assert_eq!(attributes.get("output.value"), Some(&Value::from("two")));
    assert_eq!(
        attributes.get("llm.output_messages.1.message.content"),
        Some(&Value::from("two"))
    );
}

#[test]
fn choices_missing_message_writes_empty_role_and_content() {
    let attributes = response_attributes(r#"{"choices":[{}]}"#);
    assert_eq!(attributes.get("output.value"), Some(&Value::from("")));
    assert_eq!(
        attributes.get("llm.output_messages.0.message.role"),
        Some(&Value::from(""))
    );
    assert_eq!(
        attributes.get("llm.output_messages.0.message.content"),
        Some(&Value::from(""))
    );
}

#[test]
fn choices_not_an_object_response_writes_nothing() {
    assert!(response_attributes("[1,2]").is_empty());
}

#[test]
fn media_image_url_and_b64_json_data_url() {
    let attributes = response_attributes(
        r#"{"data":[{"url":"https://example.com/a.png"},{"b64_json":"QUJD"}]}"#,
    );
    assert_eq!(
        attributes.get("image.url.0"),
        Some(&Value::from("https://example.com/a.png"))
    );
    assert_eq!(
        attributes.get("image.url.1"),
        Some(&Value::from("data:image/png;base64,QUJD"))
    );
    assert_eq!(
        attributes.get("output.value"),
        Some(&Value::from("https://example.com/a.png"))
    );
}

#[test]
fn media_audio_url_fields_and_b64_json() {
    let attributes = response_attributes(
        r#"{"audio":[{"url":"https://example.com/a.wav","mime_type":"audio/wav","transcript":"hello"},{"b64_json":"QUJD","transcript":"world"},{"mime_type":"audio/mp3"}]}"#,
    );
    assert_eq!(
        attributes.get("audio.url.0"),
        Some(&Value::from("https://example.com/a.wav"))
    );
    assert_eq!(
        attributes.get("audio.mime_type.0"),
        Some(&Value::from("audio/wav"))
    );
    assert_eq!(
        attributes.get("audio.transcript.0"),
        Some(&Value::from("hello"))
    );
    assert_eq!(
        attributes.get("audio.url.1"),
        Some(&Value::from("data:audio/wav;base64,QUJD"))
    );
    assert_eq!(
        attributes.get("audio.transcript.1"),
        Some(&Value::from("world"))
    );
    assert_eq!(
        attributes.get("audio.mime_type.2"),
        Some(&Value::from("audio/mp3"))
    );
    assert!(!attributes.contains_key("audio.url.2"));
    assert_eq!(
        attributes.get("output.value"),
        Some(&Value::from("https://example.com/a.wav"))
    );
}

#[test]
fn embedding_vector_is_written_as_python_list_text() {
    let attributes = response_attributes(
        r#"{"data":[{"embedding":[0.1,0.2,0.5],"text":"first"},{"embedding":[1,2]}]}"#,
    );
    assert_eq!(
        attributes.get("output.value"),
        Some(&Value::from("[0.1, 0.2, 0.5]"))
    );
    assert_eq!(
        attributes.get("embedding.vector.0"),
        Some(&Value::from("[0.1, 0.2, 0.5]"))
    );
    assert_eq!(
        attributes.get("embedding.vector.1"),
        Some(&Value::from("[1, 2]"))
    );
    assert_eq!(
        attributes.get("embedding.text.0"),
        Some(&Value::from("first"))
    );
    assert!(!attributes.contains_key("embedding.text.1"));
}

#[test]
fn embedding_text_is_written_per_index() {
    let attributes = response_attributes(
        r#"{"data":[{"embedding":[1],"text":"first"},{"embedding":[2],"text":"second"}]}"#,
    );
    assert_eq!(
        attributes.get("embedding.text.0"),
        Some(&Value::from("first"))
    );
    assert_eq!(
        attributes.get("embedding.text.1"),
        Some(&Value::from("second"))
    );
}

#[test]
fn structured_reasoning_summary_last_wins_and_message_text() {
    let attributes = response_attributes(
        r#"{"output":[{"type":"reasoning","summary":[{"text":"first"},{"text":"second"}]},{"type":"message","role":"assistant","content":[{"type":"output_text","text":"answer"}]}]}"#,
    );
    assert_eq!(
        attributes.get("llm.output_messages.0.message.reasoning_summary"),
        Some(&Value::from("second"))
    );
    assert_eq!(attributes.get("output.value"), Some(&Value::from("answer")));
    assert_eq!(
        attributes.get("llm.output_messages.1.message.content"),
        Some(&Value::from("answer"))
    );
    assert_eq!(
        attributes.get("llm.output_messages.1.message.role"),
        Some(&Value::from("assistant"))
    );
}

#[test]
fn structured_message_without_role_defaults_to_assistant() {
    let attributes = response_attributes(
        r#"{"output":[{"type":"message","content":[{"type":"output_text","text":"hi"}]}]}"#,
    );
    assert_eq!(
        attributes.get("llm.output_messages.0.message.role"),
        Some(&Value::from("assistant"))
    );
    assert_eq!(attributes.get("output.value"), Some(&Value::from("hi")));
}

#[test]
fn structured_empty_message_content_writes_empty_strings() {
    let attributes = response_attributes(r#"{"output":[{"type":"message","content":[]}]}"#);
    assert_eq!(attributes.get("output.value"), Some(&Value::from("")));
    assert_eq!(
        attributes.get("llm.output_messages.0.message.content"),
        Some(&Value::from(""))
    );
    assert_eq!(
        attributes.get("llm.output_messages.0.message.role"),
        Some(&Value::from("assistant"))
    );
}

#[test]
fn usage_chat_counts_are_written() {
    let attributes = response_attributes(
        r#"{"usage":{"total_tokens":30,"completion_tokens":20,"prompt_tokens":10}}"#,
    );
    assert_eq!(
        attributes,
        expected(&[
            ("llm.token_count.total", Value::I64(30)),
            ("llm.token_count.completion", Value::I64(20)),
            ("llm.token_count.prompt", Value::I64(10)),
        ])
    );
}

#[test]
fn usage_responses_api_fallbacks() {
    let attributes = response_attributes(
        r#"{"usage":{"total_tokens":30,"output_tokens":20,"input_tokens":10,"output_tokens_details":{"reasoning_tokens":5},"input_tokens_details":{"cached_tokens":3,"audio_tokens":2}}}"#,
    );
    assert_eq!(
        attributes,
        expected(&[
            ("llm.token_count.total", Value::I64(30)),
            ("llm.token_count.completion", Value::I64(20)),
            ("llm.token_count.prompt", Value::I64(10)),
            (
                "llm.token_count.completion_details.reasoning",
                Value::I64(5)
            ),
            ("llm.token_count.prompt_details.cache_read", Value::I64(3)),
            ("llm.token_count.prompt_details.audio", Value::I64(2)),
        ])
    );
}

#[test]
fn usage_cache_read_and_write() {
    let attributes = response_attributes(
        r#"{"usage":{"total_tokens":30,"cache_read_input_tokens":7,"cache_creation_input_tokens":4}}"#,
    );
    assert_eq!(
        attributes,
        expected(&[
            ("llm.token_count.total", Value::I64(30)),
            ("llm.token_count.prompt_details.cache_read", Value::I64(7)),
            ("llm.token_count.prompt_details.cache_write", Value::I64(4)),
        ])
    );
}

#[test]
fn usage_zero_counts_are_skipped_but_total_is_written() {
    let attributes = response_attributes(
        r#"{"usage":{"total_tokens":0,"completion_tokens":0,"output_tokens":0,"prompt_tokens":0,"input_tokens":0,"completion_tokens_details":{"reasoning_tokens":0},"prompt_tokens_details":{"cached_tokens":0,"audio_tokens":0},"cache_creation_input_tokens":0}}"#,
    );
    assert_eq!(
        attributes,
        expected(&[("llm.token_count.total", Value::I64(0))])
    );
}

#[test]
fn usage_zero_primary_falls_back_to_secondary_name() {
    let attributes = response_attributes(
        r#"{"usage":{"total_tokens":7,"completion_tokens":0,"output_tokens":7}}"#,
    );
    assert_eq!(
        attributes.get("llm.token_count.completion"),
        Some(&Value::I64(7))
    );
}

#[test]
fn usage_missing_total_writes_empty_string() {
    let attributes = response_attributes(r#"{"usage":{"prompt_tokens":5}}"#);
    assert_eq!(
        attributes,
        expected(&[
            ("llm.token_count.total", Value::from("")),
            ("llm.token_count.prompt", Value::I64(5)),
        ])
    );
}

#[test]
fn usage_empty_object_writes_nothing() {
    assert!(response_attributes(r#"{"usage":{}}"#).is_empty());
}

#[test]
fn text_string_is_returned_as_is() {
    assert_eq!(coerce_text(&json(r#""hello""#)), Some("hello".to_owned()));
}

#[test]
fn text_parts_join_strings_and_text_fields() {
    let value = json(
        r#"["a",{"text":"b"},{"input_text":"c"},{"text":"d","input_text":"e"},{"text":"","input_text":"f"}]"#,
    );
    assert_eq!(coerce_text(&value), Some("a\nb\nc\nd\nf".to_owned()));
}

#[test]
fn text_parts_without_text_return_none() {
    assert_eq!(
        coerce_text(&json(
            r#"[{"type":"text","text":""},{"type":"text"},{"text":7}]"#
        )),
        None
    );
}

#[rstest::rstest]
#[case::null("null")]
#[case::number("7")]
#[case::boolean("true")]
#[case::object(r#"{"text":"x"}"#)]
fn text_other_values_return_none(#[case] raw: &str) {
    assert_eq!(coerce_text(&json(raw)), None);
}
