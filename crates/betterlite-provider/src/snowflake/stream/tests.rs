use crate::snowflake::tests::support::{SequenceIds, WordCounter, parsed};

use super::{SnowflakeChunk, SnowflakeStreamingHandler, get_model_response_iterator};

fn handler() -> SnowflakeStreamingHandler<(), (), WordCounter> {
    let ids = SequenceIds::default();
    get_model_response_iterator((), false, None, WordCounter, &ids)
}

fn openai(source: &str) -> super::OpenAiStreamChunk {
    let SnowflakeChunk::OpenAi(chunk) = handler().chunk_parser(&parsed(source)).unwrap() else {
        panic!("the chunk routed to the Anthropic parser");
    };
    chunk
}

#[test]
fn a_choices_chunk_reads_its_content_and_finish_reason() {
    let chunk = openai(
        r#"{"choices":[{"index":1,"delta":{"content":"hello","reasoning_content":"ignored"},"finish_reason":null}]}"#,
    );
    assert_eq!(chunk.text, "hello");
    assert_eq!(chunk.finish_reason, "");
    assert!(!chunk.is_finished);
    assert_eq!(chunk.index, 1);
    assert_eq!(chunk.usage, None);
    assert_eq!(chunk.tool_use, None);
}

#[test]
fn a_finish_reason_marks_the_chunk_finished() {
    let chunk = openai(r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#);
    assert_eq!(chunk.finish_reason, "stop");
    assert!(chunk.is_finished);
    assert_eq!(chunk.index, 0);
}

#[test]
fn empty_choices_give_the_empty_chunk_and_no_usage() {
    let empty = openai(r#"{"choices":[]}"#);
    assert_eq!(empty.text, "");
    assert!(!empty.is_finished);
    assert_eq!(empty.finish_reason, "");
    assert_eq!(empty.usage, None);
    assert_eq!(empty.index, 0);
    assert_eq!(empty.tool_use, None);
    let missing = openai(r#"{"choices":null,"usage":{"prompt_tokens":1}}"#);
    assert_eq!(missing, empty);
}

#[test]
fn only_the_first_tool_call_of_the_first_choice_is_read() {
    let chunk = openai(
        r#"{"choices":[{"delta":{"tool_calls":[{"id":"call_1","function":{"name":"f","arguments":"{}"},"index":2},{"id":"call_2","function":{"name":"g"}}]}},{"delta":{"content":"ignored"}}]}"#,
    );
    let tool = chunk.tool_use.unwrap();
    assert_eq!(tool.id, "call_1");
    assert_eq!(tool.kind, "function");
    assert_eq!(tool.function.name, "f");
    assert_eq!(tool.function.arguments, "{}");
    assert_eq!(tool.index, 2);
}

#[test]
fn a_tool_call_without_fields_defaults_to_empty_strings() {
    let chunk = openai(r#"{"choices":[{"delta":{"tool_calls":[{}]}}]}"#);
    let tool = chunk.tool_use.unwrap();
    assert_eq!(tool.id, "");
    assert_eq!(tool.function.name, "");
    assert_eq!(tool.function.arguments, "");
    assert_eq!(tool.index, 0);
}

#[test]
fn an_empty_tool_call_list_has_no_tool_use() {
    let chunk = openai(r#"{"choices":[{"delta":{"tool_calls":[]}}]}"#);
    assert_eq!(chunk.tool_use, None);
}

#[test]
fn a_chunk_with_choices_and_a_type_takes_the_openai_path() {
    let chunk = openai(r#"{"type":"message","choices":[{"delta":{"content":"x"}}]}"#);
    assert_eq!(chunk.text, "x");
}

#[test]
fn an_anthropic_event_takes_the_anthropic_parser() {
    let mut parser = handler();
    let started = parser
        .chunk_parser(&parsed(
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":"Hello"}}"#,
        ))
        .unwrap();
    let SnowflakeChunk::Anthropic(start) = started else {
        panic!("the event routed to the OpenAI path");
    };
    assert_eq!(start.choices[0].delta.content, "Hello");
    let changed = parser
        .chunk_parser(&parsed(
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":" world"}}"#,
        ))
        .unwrap();
    let SnowflakeChunk::Anthropic(delta) = changed else {
        panic!("the event routed to the OpenAI path");
    };
    assert_eq!(delta.choices[0].delta.content, " world");
}

#[test]
fn a_malformed_choices_field_is_an_error_naming_the_field() {
    let mut parser = handler();
    let error = parser
        .chunk_parser(&parsed(r#"{"choices":{"index":0}}"#))
        .unwrap_err();
    assert_eq!(error.to_string(), "the stream chunk choices must be a list");
}

#[test]
fn a_non_string_content_is_an_error_naming_the_field() {
    let mut parser = handler();
    let error = parser
        .chunk_parser(&parsed(r#"{"choices":[{"delta":{"content":5}}]}"#))
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "the stream chunk field content must be a string"
    );
}
