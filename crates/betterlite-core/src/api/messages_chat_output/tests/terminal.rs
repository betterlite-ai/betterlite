#![cfg(test)]

use crate::api::messages_chat_output::error::ChatOutputError;
use crate::api::messages_chat_output::output::StreamFrame;
use crate::api::messages_chat_output::stream::{delta, terminal_frame};

use super::support::{assert_rendered, terminal};

#[test]
fn terminal_chunk_projects_stop_reason_and_usage() {
    let record = terminal(
        r#"{"choices": [{"finish_reason": "stop", "delta": {"content": "ignored"}}],
            "usage": {"prompt_tokens": 100, "completion_tokens": 7, "cache_read_input_tokens": 20,
                      "cache_creation_input_tokens": 30},
            "hidden_usage": {"prompt_tokens": 1, "completion_tokens": 2}}"#,
    );
    assert_rendered(
        &terminal_frame(&record).expect("projects a frame"),
        r#"{"type": "message_delta", "delta": {"stop_reason": "end_turn"},
            "usage": {"input_tokens": 50, "output_tokens": 7,
                      "cache_creation_input_tokens": 30, "cache_read_input_tokens": 20}}"#,
    );
}

#[test]
fn terminal_chunk_uses_hidden_usage_next() {
    let record = terminal(
        r#"{"choices": [{"finish_reason": "length"}],
            "hidden_usage": {"prompt_tokens": 10, "completion_tokens": 4,
                             "_cache_read_input_tokens": 3}}"#,
    );
    assert_rendered(
        &terminal_frame(&record).expect("projects a frame"),
        r#"{"type": "message_delta", "delta": {"stop_reason": "max_tokens"},
            "usage": {"input_tokens": 7, "output_tokens": 4, "cache_read_input_tokens": 3}}"#,
    );
}

#[test]
fn terminal_chunk_zeroes_missing_usage() {
    let record = terminal(r#"{"choices": [{"finish_reason": "tool_calls"}]}"#);
    assert_rendered(
        &terminal_frame(&record).expect("projects a frame"),
        r#"{"type": "message_delta", "delta": {"stop_reason": "tool_use"},
            "usage": {"input_tokens": 0, "output_tokens": 0}}"#,
    );
}

#[test]
fn terminal_chunk_treats_an_empty_finish_reason_as_terminal() {
    let record = terminal(r#"{"choices": [{"finish_reason": ""}]}"#);
    assert_rendered(
        &terminal_frame(&record).expect("projects a frame"),
        r#"{"type": "message_delta", "delta": {"stop_reason": "end_turn"},
            "usage": {"input_tokens": 0, "output_tokens": 0}}"#,
    );
}

#[test]
fn terminal_chunk_attaches_truthy_edits_only() {
    let record = terminal(
        r#"{"choices": [{"finish_reason": "stop"}],
            "applied_edits": [{"type": "clear_tool_uses_20250919", "cleared_input_tokens": 5}]}"#,
    );
    assert_rendered(
        &terminal_frame(&record).expect("projects a frame"),
        r#"{"type": "message_delta", "delta": {"stop_reason": "end_turn"},
            "usage": {"input_tokens": 0, "output_tokens": 0},
            "context_management": {"applied_edits": [
                {"type": "clear_tool_uses_20250919", "cleared_input_tokens": 5}]}}"#,
    );
    let empty = terminal(r#"{"choices": [{"finish_reason": "stop"}], "applied_edits": []}"#);
    let frame = terminal_frame(&empty).expect("projects a frame");
    assert!(matches!(
        frame,
        StreamFrame::MessageDelta {
            context_management: None,
            ..
        }
    ));
}

#[test]
fn nonterminal_chunk_projects_a_content_block_delta() {
    let record = terminal(r#"{"index": 4, "choices": [{"delta": {"content": "hi"}}]}"#);
    assert_rendered(
        &terminal_frame(&record).expect("projects a frame"),
        r#"{"type": "content_block_delta", "index": 4,
            "delta": {"type": "text_delta", "text": "hi"}}"#,
    );
}

#[test]
fn nonterminal_chunk_carries_delta_errors() {
    let record = terminal(
        r#"{"index": 0, "choices": [{"delta": {"thinking_blocks": [
            {"type": "thinking", "thinking": true}]}}]}"#,
    );
    assert!(matches!(
        terminal_frame(&record),
        Err(ChatOutputError::NonTextThinkingField { field: "thinking" })
    ));
}

#[test]
fn zero_choice_chunks_fail_without_a_terminal() {
    let record = terminal(r#"{"choices": [], "usage": {"prompt_tokens": 1}}"#);
    assert!(matches!(
        terminal_frame(&record),
        Err(ChatOutputError::NoChoices)
    ));
    assert_rendered(
        &delta(&[]).expect("projects a delta"),
        r#"{"type": "text_delta", "text": ""}"#,
    );
}
