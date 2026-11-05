#![cfg(test)]
use super::lines::Lines;
use super::wire::Usage;
use super::{
    CompatibleLineChunk, Mode, Outcome, ReceiveError, StreamError, decode_legacy_compatible_line,
    receive_failure,
};
use futures_util::StreamExt as _;
use futures_util::stream;
use rstest::rstest;
use serde_json::Value;
use std::error::Error as _;
use std::path::PathBuf;
use std::sync::Mutex;
use std::{fs, io};
use tracing::subscriber::with_default;

#[test]
fn compatible_line_sync_done_is_empty_and_async_done_ends() {
    let Outcome::Chunk(chunk) = decode_legacy_compatible_line("data: [DONE]", Mode::Sync) else {
        panic!("sync DONE is parsed as an empty chunk")
    };
    assert!(!chunk.is_finished);
    assert_eq!(
        decode_legacy_compatible_line("data: [DONE]", Mode::Async),
        Outcome::End
    );
}

#[test]
fn compatible_line_projects_first_choice_and_tool() {
    let line = r#"{"choices":[{"index":9,"delta":{"content":"hi","tool_calls":[{"index":4,"id":"t","function":{"name":"f","arguments":"{}"}},{"index":5,"id":"u","function":{"name":"g","arguments":"x"}}]},"finish_reason":""},{"delta":{"content":"ignored"}}],"id":"drop"}"#;
    let Outcome::Chunk(chunk) = decode_legacy_compatible_line(line, Mode::Sync) else {
        panic!("chunk")
    };
    assert_eq!(chunk.text, "hi");
    assert_eq!(chunk.index, 0);
    assert!(!chunk.is_finished);
    assert_eq!(chunk.tool_use.unwrap().id.unwrap(), "t");
}

#[test]
fn compatible_line_usage_empty_choices_stays_unfinished() {
    let line = r#"{"choices":[],"usage":{"prompt_tokens":null,"completion_tokens":2,"total_tokens":2},"id":"drop"}"#;
    let Outcome::Chunk(chunk) = decode_legacy_compatible_line(line, Mode::Sync) else {
        panic!("chunk")
    };
    assert!(!chunk.is_finished);
    assert_eq!(chunk.usage.unwrap().prompt_tokens, 0_i64);
}

#[test]
fn compatible_line_ignores_null_finish_reason_and_usage() {
    let line = r#"{"choices":[{"delta":{"content":"answer"},"finish_reason":null}],"usage":null}"#;
    let Outcome::Chunk(chunk) = decode_legacy_compatible_line(line, Mode::Sync) else {
        panic!("valid chunk")
    };
    assert!(!chunk.is_finished);
    assert_eq!(chunk.usage, None);
}

#[rstest]
#[case::length("max_tokens", "length")]
#[case::tool_calls("tool_use", "tool_calls")]
#[case::content_filter("SAFETY", "content_filter")]
#[case::unmapped("whatever", "stop")]
fn compatible_line_maps_finish_reason_with_the_shared_table(
    #[case] raw: &str,
    #[case] expected: &str,
) {
    let line = format!(r#"{{"choices":[{{"delta":{{}},"finish_reason":"{raw}"}}]}}"#);
    let Outcome::Chunk(chunk) = decode_legacy_compatible_line(&line, Mode::Sync) else {
        panic!("chunk")
    };
    assert!(chunk.is_finished);
    assert_eq!(chunk.finish_reason, expected);
}

#[rstest]
#[case::object_arguments(r#"{"arguments":{"a":1}}"#, r#"{"a":1}"#)]
#[case::parameters_without_arguments(r#"{"parameters":{"a":1}}"#, r#"{"a":1}"#)]
#[case::parameters_with_null_arguments(r#"{"arguments":null,"parameters":{"a":1}}"#, r#"{"a":1}"#)]
#[case::string_arguments_win(r#"{"arguments":"raw","parameters":{"a":1}}"#, "raw")]
#[case::object_arguments_win(r#"{"arguments":{"b":2},"parameters":{"a":1}}"#, r#"{"b":2}"#)]
#[case::non_object_parameters_are_ignored(r#"{"parameters":"text"}"#, "")]
#[case::empty_object(r#"{"arguments":{}}"#, "{}")]
fn compatible_line_tool_arguments_take_objects_and_parameters(
    #[case] function: &str,
    #[case] expected: &str,
) {
    let line = format!(
        r#"{{"choices":[{{"delta":{{"content":"hi","tool_calls":[{{"index":2,"id":"t","function":{function}}}]}}}}]}}"#
    );
    let Outcome::Chunk(chunk) = decode_legacy_compatible_line(&line, Mode::Sync) else {
        panic!("chunk")
    };
    assert_eq!(chunk.text, "hi");
    assert_eq!(chunk.tool_use.unwrap().function.arguments, expected);
}

#[rstest]
#[case::type_custom(r#"{"type":"custom","id":"c","custom":{"name":"n","input":"i"}}"#)]
#[case::custom_key_only(r#"{"id":"c","custom":{"name":"n","input":"i"}}"#)]
#[case::custom_with_function(
    r#"{"type":"custom","custom":{"input":"i"},"function":{"arguments":"x"}}"#
)]
#[case::custom_with_null_function(r#"{"custom":{},"function":null}"#)]
fn custom_first_tool_call_keeps_text_finish_reason_and_usage(#[case] tool_call: &str) {
    let line = format!(
        r#"{{"choices":[{{"delta":{{"content":"hi","tool_calls":[{tool_call},{{"function":{{"arguments":"second"}}}}]}},"finish_reason":"tool_calls"}}],"usage":{{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}}}"#
    );
    let Outcome::Chunk(chunk) = decode_legacy_compatible_line(&line, Mode::Sync) else {
        panic!("chunk")
    };
    assert_eq!(chunk.text, "hi");
    assert_eq!(chunk.tool_use, None);
    assert!(chunk.is_finished);
    assert_eq!(chunk.finish_reason, "tool_calls");
    assert_eq!(chunk.usage.unwrap().total_tokens, 3_i64);
}

#[rstest]
#[case::arguments_number(r#"{"function":{"arguments":1}}"#)]
#[case::arguments_array(r#"{"function":{"arguments":[1]}}"#)]
#[case::name_number(r#"{"function":{"arguments":"x","name":3}}"#)]
#[case::type_custom_without_payload(r#"{"type":"custom","function":{"arguments":"x"}}"#)]
#[case::type_custom_null_payload(r#"{"type":"custom","custom":null}"#)]
#[case::payload_name_number(r#"{"custom":{"name":3}}"#)]
#[case::payload_not_object(r#"{"custom":"text"}"#)]
#[case::custom_index_text(r#"{"custom":{},"index":"x"}"#)]
fn invalid_tool_call_empties_the_chunk(#[case] tool_call: &str) {
    let line = format!(
        r#"{{"choices":[{{"delta":{{"content":"hi","tool_calls":[{tool_call}]}},"finish_reason":"stop"}}]}}"#
    );
    assert_eq!(
        decode_legacy_compatible_line(&line, Mode::Sync),
        decode_legacy_compatible_line("", Mode::Sync)
    );
}

#[test]
fn second_custom_tool_call_is_not_projected_and_does_not_fail() {
    let line = r#"{"choices":[{"delta":{"tool_calls":[{"id":"f","function":{"arguments":"x"}},{"custom":{"name":"n"}}]}}]}"#;
    let Outcome::Chunk(chunk) = decode_legacy_compatible_line(line, Mode::Sync) else {
        panic!("chunk")
    };
    assert_eq!(chunk.tool_use.unwrap().id.unwrap(), "f");
}

#[test]
fn compatible_line_blank_and_invalid_are_empty_chunks() {
    assert!(matches!(
        decode_legacy_compatible_line("  ", Mode::Async),
        Outcome::Chunk(_)
    ));
    assert!(matches!(
        decode_legacy_compatible_line("no json", Mode::Sync),
        Outcome::Chunk(_)
    ));
}

#[rstest]
#[case::bad_model(r#"{"model":42,"choices":[{"delta":{"content":"discard"}}]}"#)]
#[case::bad_usage(r#"{"choices":[],"usage":{"prompt_tokens":"invalid"}}"#)]
#[case::bad_tool(
    r#"{"choices":[{"delta":{"tool_calls":[{"index":"invalid","function":{"arguments":"x"}}]}}]}"#
)]
fn model_validation_failure_is_an_empty_chunk(#[case] line: &str) {
    assert_eq!(
        decode_legacy_compatible_line(line, Mode::Sync),
        decode_legacy_compatible_line("", Mode::Sync)
    );
}

#[rstest]
#[case::missing_first("[{}]")]
#[case::null_first(r#"[{"function":null}]"#)]
#[case::missing_second(r#"[{"function":{"arguments":"{}"}},{}]"#)]
#[case::null_second(r#"[{"function":{"arguments":"{}"}},{"function":null}]"#)]
fn missing_or_null_tool_function_empties_whole_chunk(
    #[case] tool_calls: &str,
    #[values(Mode::Sync, Mode::Async)] mode: Mode,
) {
    let line = format!(
        r#"{{"choices":[{{"delta":{{"content":"hi","tool_calls":{tool_calls}}},"finish_reason":"stop"}}],"usage":{{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}}}"#
    );
    assert_eq!(
        decode_legacy_compatible_line(&line, mode),
        Outcome::Chunk(CompatibleLineChunk {
            text: String::new(),
            tool_use: None,
            is_finished: false,
            finish_reason: String::new(),
            usage: None,
            index: 0,
        })
    );
}

#[test]
fn ordinary_usage_has_model_defaults() {
    let Outcome::Chunk(chunk) = decode_legacy_compatible_line(
        r#"{"usage":{"prompt_tokens":null,"completion_tokens":3},"choices":[{}]}"#,
        Mode::Async,
    ) else {
        panic!("chunk")
    };
    assert_eq!(
        chunk.usage.unwrap(),
        Usage {
            prompt_tokens: 0,
            completion_tokens: 3,
            total_tokens: 0,
        }
    );
}

#[rstest]
#[case::sync_done("data: [DONE]")]
#[case::invalid_json("data: nope")]
#[case::invalid_model(r#"{"id":42}"#)]
#[case::missing_function(r#"{"choices":[{"delta":{"tool_calls":[{}]}}]}"#)]
#[case::null_function(r#"{"choices":[{"delta":{"tool_calls":[{"function":null}]}}]}"#)]
fn parse_fallback_emits_debug_diagnostic(#[case] line: &str) {
    let file = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(Mutex::new(file.reopen().unwrap()))
        .finish();
    let outcome = with_default(subscriber, || {
        decode_legacy_compatible_line(line, Mode::Sync)
    });
    assert_eq!(outcome, decode_legacy_compatible_line("", Mode::Sync));
    assert!(
        fs::read_to_string(file.path())
            .unwrap()
            .contains("Error parsing chunk")
    );
}

#[rstest]
#[case::sync_other(Mode::Sync, "connection reset")]
#[case::async_other(Mode::Async, "Error receiving chunk from stream: connection reset")]
fn compatible_line_other_receive_errors_keep_mode_behavior(
    #[case] mode: Mode,
    #[case] expected: &str,
) {
    assert_eq!(
        receive_failure(
            ReceiveError::Other(io::Error::other("connection reset")),
            mode
        )
        .to_string(),
        expected
    );
}

#[rstest]
fn replays_the_compatible_line_corpus(#[files("fuzz/corpus/compatible_line/*")] path: PathBuf) {
    let line = fs::read_to_string(path).unwrap();
    assert!(matches!(
        decode_legacy_compatible_line(&line, Mode::Async),
        Outcome::Chunk(_) | Outcome::End
    ));
}

#[rstest]
#[case::sync(Mode::Sync)]
#[case::async_mode(Mode::Async)]
fn value_receive_errors_keep_the_source(#[case] mode: Mode) {
    let error = receive_failure(ReceiveError::Value(io::Error::other("bad value")), mode);
    assert!(matches!(error, StreamError::Runtime(_)));
    assert_eq!(
        error.to_string(),
        "Error receiving chunk from stream: bad value"
    );
    assert_eq!(error.source().unwrap().to_string(), "bad value");
}

#[rstest]
#[case::sync(Mode::Sync, false)]
#[case::async_mode(Mode::Async, true)]
fn other_receive_error_category(#[case] mode: Mode, #[case] runtime: bool) {
    let error = receive_failure(ReceiveError::Other(io::Error::other("reset")), mode);
    assert_eq!(matches!(error, StreamError::Runtime(_)), runtime);
    assert_eq!(error.source().unwrap().to_string(), "reset");
}

#[test]
fn projects_complete_tool_and_usage_without_provider_fields() {
    let outcome = decode_legacy_compatible_line(
        r#"data:{"choices":[{"index":9,"delta":{"content":"hi","role":"assistant","tool_calls":[{"id":"t","index":4,"function":{"name":"f","arguments":"{}"}},{"function":{"arguments":"ignored"}}]},"finish_reason":"stop"},{"delta":{"content":"ignored"}}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3,"extra":100}}"#,
        Mode::Async,
    );
    let expected = serde_json::from_str::<Value>(
        r#"{"kind":"chunk","value":{
            "text":"hi","tool_use":{"id":"t","type":"function","function":{"name":"f","arguments":"{}"},"index":4},
            "is_finished":true,"finish_reason":"stop","usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3},"index":0
        }}"#,
    ).unwrap();
    assert_eq!(serde_json::to_value(outcome).unwrap(), expected);
}

#[test]
fn missing_tool_index_and_arguments_take_model_defaults() {
    let Outcome::Chunk(chunk) = decode_legacy_compatible_line(
        r#"{"choices":[{"delta":{"tool_calls":[{"function":{}}]}}]}"#,
        Mode::Sync,
    ) else {
        panic!("chunk")
    };
    let tool = chunk.tool_use.unwrap();
    assert_eq!(tool.index, 0);
    assert_eq!(tool.id, None);
    assert_eq!(tool.function.name, None);
    assert_eq!(tool.function.arguments, "");
}

type Source = Vec<Result<String, ReceiveError<io::Error>>>;

fn drain_sync(source: Source) -> Vec<Result<CompatibleLineChunk, StreamError<io::Error>>> {
    Lines::from(source.into_iter()).collect()
}

async fn drain_async(source: Source) -> Vec<Result<CompatibleLineChunk, StreamError<io::Error>>> {
    Lines::from(stream::iter(source)).collect().await
}

#[test]
fn sync_lines_turn_done_into_an_empty_chunk_and_end_on_exhaustion() {
    let items = drain_sync(vec![
        Ok(String::from(r#"{"choices":[{"delta":{"content":"a"}}]}"#)),
        Ok(String::from("data: [DONE]")),
        Ok(String::from(r#"{"choices":[{"delta":{"content":"b"}}]}"#)),
    ]);
    let texts: Vec<_> = items.into_iter().map(|item| item.unwrap().text).collect();
    assert_eq!(texts, ["a", "", "b"]);
}

#[test]
fn sync_lines_keep_a_blank_line_apart_from_exhaustion() {
    assert_eq!(drain_sync(vec![Ok(String::new())]).len(), 1);
    assert!(drain_sync(Vec::new()).is_empty());
}

#[tokio::test]
async fn async_lines_stop_at_done_and_leave_the_rest_of_the_source_unread() {
    let source: Source = vec![
        Ok(String::from(
            r#"{"choices":[{"delta":{"content":"before"}}]}"#,
        )),
        Ok(String::from("data: [DONE]")),
        Ok(String::from(
            r#"{"choices":[{"delta":{"content":"after"}}]}"#,
        )),
    ];
    let mut lines = Lines::from(stream::iter(source));
    assert_eq!(lines.next().await.unwrap().unwrap().text, "before");
    assert!(lines.next().await.is_none());
    assert!(lines.next().await.is_none());
    assert_eq!(lines.source.count().await, 1);
}

#[tokio::test]
async fn async_lines_emit_blank_chunks_and_end_on_exhaustion() {
    assert_eq!(
        drain_async(vec![Ok(String::new()), Ok(String::from("  "))])
            .await
            .len(),
        2
    );
    assert!(drain_async(Vec::new()).await.is_empty());
}

#[rstest]
#[case::sync(Mode::Sync, "connection reset", false)]
#[case::async_mode(
    Mode::Async,
    "Error receiving chunk from stream: connection reset",
    true
)]
#[tokio::test]
async fn other_receive_errors_end_the_stream_with_the_mode_error(
    #[case] mode: Mode,
    #[case] expected: &str,
    #[case] runtime: bool,
) {
    let source: Source = vec![
        Err(ReceiveError::Other(io::Error::other("connection reset"))),
        Ok(String::from(
            r#"{"choices":[{"delta":{"content":"after"}}]}"#,
        )),
    ];
    let items = match mode {
        Mode::Sync => drain_sync(source),
        Mode::Async => drain_async(source).await,
    };
    assert_eq!(items.len(), 1);
    let error = items.into_iter().next().unwrap().unwrap_err();
    assert_eq!(matches!(error, StreamError::Runtime(_)), runtime);
    assert_eq!(error.to_string(), expected);
}

#[rstest]
#[case::sync(Mode::Sync)]
#[case::async_mode(Mode::Async)]
#[tokio::test]
async fn value_receive_errors_end_the_stream_as_runtime_errors(#[case] mode: Mode) {
    let source: Source = vec![
        Ok(String::from(
            r#"{"choices":[{"delta":{"content":"before"}}]}"#,
        )),
        Err(ReceiveError::Value(io::Error::other("bad value"))),
        Ok(String::from(
            r#"{"choices":[{"delta":{"content":"after"}}]}"#,
        )),
    ];
    let items = match mode {
        Mode::Sync => drain_sync(source),
        Mode::Async => drain_async(source).await,
    };
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].as_ref().unwrap().text, "before");
    let error = items.into_iter().nth(1).unwrap().unwrap_err();
    assert_eq!(
        error.to_string(),
        "Error receiving chunk from stream: bad value"
    );
}
