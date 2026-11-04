#![cfg(test)]
use super::wire::Usage;
use super::{
    Mode, Outcome, ReceiveError, StreamError, decode_legacy_compatible_line, receive_failure,
};
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
    assert!(matches!(error, StreamError::Runtime { .. }));
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
    assert_eq!(matches!(error, StreamError::Runtime { .. }), runtime);
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
