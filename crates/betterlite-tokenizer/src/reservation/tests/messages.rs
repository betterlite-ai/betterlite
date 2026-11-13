use std::io::{Error as IoError, Result as IoResult, Write};
use std::sync::{Arc, Mutex};

use rstest::rstest;
use tokio::runtime::Builder;
use tracing::Level;
use tracing::subscriber::with_default;

use crate::reservation::count_input_tokens_for_model;
use crate::tools::{ToolLimits, format_function_definitions, parse_tool_definitions};

use super::support;

const MODEL: &str = "gpt-3.5-turbo";

#[tokio::test]
async fn a_value_error_counts_the_original_messages_as_text() {
    let directory = tempfile::tempdir().unwrap();
    let counter = support::bundled_counter(directory.path());
    let source = r#"{"messages": [{"role": "user", "content": [{"type": "input_audio", "input_audio": {"data": "AAA", "format": "wav"}}]}]}"#;
    let body = support::body(source);
    let tokens = count_input_tokens_for_model(&counter, &body, MODEL).await;
    let original = r#"{"role": "user", "content": [{"type": "input_audio", "input_audio": {"data": "AAA", "format": "wav"}}]}"#;
    let expected = support::text_tokens(&counter, MODEL, original).await;
    assert_eq!(tokens, Some(expected));
}

#[rstest]
#[case::thinking_text_only(
    r#"{"type": "thinking", "thinking": "plan the answer", "signature": "c2lnbmF0dXJl"}"#,
    "plan the answer"
)]
#[case::redacted_thinking_without_text(r#"{"type": "redacted_thinking", "data": "opaque"}"#, "")]
#[case::tool_reference_name(
    r#"{"type": "tool_reference", "tool_name": "get_weather"}"#,
    "get_weather"
)]
#[case::tool_reference_empty_name(r#"{"type": "tool_reference", "tool_name": ""}"#, "")]
#[tokio::test]
async fn thinking_and_tool_reference_blocks_count_their_text_not_the_fallback(
    #[case] block: &str,
    #[case] counted: &str,
) {
    let directory = tempfile::tempdir().unwrap();
    let counter = support::bundled_counter(directory.path());
    let with_block = format!(r#"{{"messages": [{{"role": "assistant", "content": [{block}]}}]}}"#);
    let with_text = if counted.is_empty() {
        r#"{"messages": [{"role": "assistant", "content": []}]}"#.to_owned()
    } else {
        format!(
            r#"{{"messages": [{{"role": "assistant", "content": [{{"type": "text", "text": "{counted}"}}]}}]}}"#
        )
    };
    let block_tokens =
        count_input_tokens_for_model(&counter, &support::body(&with_block), MODEL).await;
    let text_tokens =
        count_input_tokens_for_model(&counter, &support::body(&with_text), MODEL).await;
    assert!(block_tokens.is_some());
    assert_eq!(block_tokens, text_tokens);
}

#[tokio::test]
async fn a_bad_tool_calls_shape_counts_the_original_messages_as_text() {
    let directory = tempfile::tempdir().unwrap();
    let counter = support::bundled_counter(directory.path());
    let body = support::body(r#"{"messages": [{"role": "user", "tool_calls": "bad"}]}"#);
    let tokens = count_input_tokens_for_model(&counter, &body, MODEL).await;
    let expected =
        support::text_tokens(&counter, MODEL, r#"{"role": "user", "tool_calls": "bad"}"#).await;
    assert_eq!(tokens, Some(expected));
}

#[rstest]
#[case::message_is_not_an_object(r#"{"messages": ["hello"]}"#)]
#[case::messages_is_a_string(r#"{"messages": "hello"}"#)]
#[case::array_schema_without_items(
    r#"{"messages": [{"role": "user", "content": "hi"}], "tools": [{"function": {"name": "x", "parameters": {"type": "object", "properties": {"values": {"type": "array"}}}}}]}"#
)]
#[tokio::test]
async fn a_failure_that_is_not_a_value_error_leaves_the_model_out(#[case] source: &str) {
    let directory = tempfile::tempdir().unwrap();
    let counter = support::bundled_counter(directory.path());
    let body = support::body(source);
    assert_eq!(
        count_input_tokens_for_model(&counter, &body, MODEL).await,
        None
    );
}

#[rstest]
#[case::content_text_block(
    r#"{"messages": [{"role": "user", "content": [{"type": "text", "text": "hello world"}]}]}"#
)]
#[case::tool_call_arguments(
    r#"{"messages": [{"role": "user", "tool_calls": [{"function": {"arguments": "hello world"}}]}]}"#
)]
#[case::function_call_arguments(
    r#"{"messages": [{"role": "user", "function_call": {"arguments": "hello world"}}]}"#
)]
#[tokio::test]
async fn message_parts_and_call_arguments_are_counted(#[case] source: &str) {
    let directory = tempfile::tempdir().unwrap();
    let counter = support::bundled_counter(directory.path());
    let body = support::body(source);
    let expected: usize = [
        3,
        3,
        support::text_tokens(&counter, MODEL, "user").await,
        support::text_tokens(&counter, MODEL, "hello world").await,
    ]
    .into_iter()
    .sum();
    assert_eq!(
        count_input_tokens_for_model(&counter, &body, MODEL).await,
        Some(expected)
    );
}

#[tokio::test]
async fn tools_and_tool_choice_reach_the_message_counter() {
    let directory = tempfile::tempdir().unwrap();
    let counter = support::bundled_counter(directory.path());
    let tools = r#"[{"type": "function", "function": {"name": "get_weather", "description": "Get weather", "parameters": {"type": "object", "properties": {"city": {"type": "string"}}, "required": ["city"]}}}]"#;
    let source = format!(
        r#"{{"messages": [{{"role": "user", "content": "hello"}}], "tools": {tools}, "tool_choice": {{"function": {{"name": "get_weather"}}}}}}"#
    );
    let body = support::body(&source);
    let definitions = parse_tool_definitions(tools.as_bytes(), ToolLimits::default()).unwrap();
    let formatted = format_function_definitions(&definitions).unwrap();
    let expected: usize = [
        3,
        1,
        1,
        3,
        9,
        support::text_tokens(&counter, MODEL, &formatted).await,
        7,
        support::text_tokens(&counter, MODEL, "get_weather").await,
    ]
    .into_iter()
    .sum();
    assert_eq!(
        count_input_tokens_for_model(&counter, &body, MODEL).await,
        Some(expected)
    );
}

#[test]
fn errors_are_logged_without_the_request_text() {
    let directory = tempfile::tempdir().unwrap();
    let counter = support::bundled_counter(directory.path());
    let body = support::body(r#"{"messages": "SUPERSECRET-text"}"#);
    let captured = Arc::new(Mutex::new(Vec::<u8>::new()));
    let writer = SharedWriter(Arc::clone(&captured));
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    let tokens = with_default(subscriber, || {
        runtime.block_on(count_input_tokens_for_model(&counter, &body, MODEL))
    });
    assert_eq!(tokens, None);
    let logged = String::from_utf8(captured.lock().unwrap().clone()).unwrap();
    assert!(logged.contains("unable to count input tokens for budget reservation"));
    assert!(!logged.contains("SUPERSECRET"));
}

#[derive(Clone)]
struct SharedWriter(Arc<Mutex<Vec<u8>>>);

impl Write for SharedWriter {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        let length = buf.len();
        let mut captured = self
            .0
            .lock()
            .map_err(|_poisoned| IoError::other("the log buffer is poisoned"))?;
        captured.extend_from_slice(buf);
        drop(captured);
        Ok(length)
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}
