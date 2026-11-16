#![cfg(test)]

use super::{FragmentPosition, function_tool_call_fragments};
use crate::guardrails::chat_guardrail_translation::rewrites::tests::support::chunks;

fn at(chunk: usize, choice: usize, tool_call: usize) -> FragmentPosition {
    FragmentPosition {
        chunk,
        choice,
        tool_call,
    }
}

#[test]
fn tool_call_fragments_group_by_tool_call_index_across_chunks() {
    let stream = chunks(
        r#"[
        {"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "id": "a", "function": {"name": "f", "arguments": ""}}]}}]},
        {"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "function": {"arguments": "{\"x\""}}]}}]},
        {"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 1, "id": "b", "function": {"name": "g", "arguments": "{}"}}]}}]},
        {"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "function": {"arguments": ":1}"}}]}}]}]"#,
    );
    assert_eq!(
        function_tool_call_fragments(&stream),
        [
            vec![at(0, 0, 0), at(1, 0, 0), at(3, 0, 0)],
            vec![at(2, 0, 0)]
        ]
    );
}

#[test]
fn tool_call_fragments_list_the_groups_in_index_order() {
    let stream = chunks(
        r#"[{"choices": [{"delta": {"tool_calls": [
            {"index": 1, "id": "b", "function": {"name": "g"}},
            {"index": 0, "id": "a", "function": {"name": "f"}}]}}]}]"#,
    );
    assert_eq!(
        function_tool_call_fragments(&stream),
        [vec![at(0, 0, 1)], vec![at(0, 0, 0)]]
    );
}

#[test]
fn tool_call_fragments_drop_an_index_that_never_had_both_an_id_and_a_name() {
    let stream = chunks(
        r#"[
        {"choices": [{"delta": {"tool_calls": [{"index": 0, "id": "a", "function": {"arguments": "{}"}}]}}]},
        {"choices": [{"delta": {"tool_calls": [{"index": 1, "function": {"name": "g"}}]}}]},
        {"choices": [{"delta": {"tool_calls": [{"index": 2, "id": "", "function": {"name": ""}}]}}]},
        {"choices": [{"delta": {"tool_calls": [{"index": 3, "id": "c"}, {"index": 3, "function": {"name": "h"}}]}}]}]"#,
    );
    assert_eq!(
        function_tool_call_fragments(&stream),
        [vec![at(3, 0, 0), at(3, 0, 1)]]
    );
}

#[test]
fn tool_call_fragments_treat_a_missing_index_as_zero_and_skip_non_object_fragments() {
    let stream = chunks(
        r#"[{"choices": [{"delta": {"tool_calls": [
            "junk", {"id": "a", "function": {"name": "f"}}]}}]}]"#,
    );
    assert_eq!(function_tool_call_fragments(&stream), [vec![at(0, 0, 1)]]);
}

#[test]
fn tool_call_fragments_ignore_chunks_without_tool_calls() {
    let stream = chunks(
        r#"[{"choices": [{"delta": {"content": "hi"}}, {"message": {"content": "x"}}]}, {"model": "m"}]"#,
    );
    assert!(function_tool_call_fragments(&stream).is_empty());
}
