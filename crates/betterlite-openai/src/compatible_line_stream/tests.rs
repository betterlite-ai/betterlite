#![cfg(test)]
use super::{Mode, Outcome, decode_legacy_compatible_line};
use rstest::rstest;
use std::fs;
use std::path::PathBuf;

#[test]
fn compatible_line_sync_done_is_empty_and_async_done_ends() {
    assert!(matches!(
        decode_legacy_compatible_line("data: [DONE]", Mode::Sync),
        Outcome::Chunk(_)
    ));
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
    assert_eq!(chunk.tool_use.unwrap()["id"], "t");
}

#[test]
fn compatible_line_usage_empty_choices_stays_unfinished() {
    let line = r#"{"choices":[],"usage":{"prompt_tokens":null,"completion_tokens":2,"total_tokens":2},"id":"drop"}"#;
    let Outcome::Chunk(chunk) = decode_legacy_compatible_line(line, Mode::Sync) else {
        panic!("chunk")
    };
    assert!(!chunk.is_finished);
    assert_eq!(chunk.usage.unwrap()["prompt_tokens"], 0_i32);
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
fn replays_the_compatible_line_corpus(#[files("fuzz/corpus/compatible_line/*")] path: PathBuf) {
    let line = fs::read_to_string(path).unwrap();
    assert!(matches!(
        decode_legacy_compatible_line(&line, Mode::Async),
        Outcome::Chunk(_) | Outcome::End
    ));
}
