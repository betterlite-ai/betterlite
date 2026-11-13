#![cfg(test)]

use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;

use super::apply_input_tool_calls;
use crate::guardrails::chat_guardrail_translation::extraction::apply_error::ApplyError;
use crate::guardrails::chat_guardrail_translation::extraction::fixtures::{json, object, values};

fn rewrite(
    source: JsonValue,
    guardrailed: &[JsonValue],
    mappings: &[(usize, usize)],
) -> (Result<(), ApplyError>, JsonValue) {
    let mut messages = values(source);
    let tool_calls: Vec<_> = guardrailed.iter().cloned().map(object).collect();
    let result = apply_input_tool_calls(&mut messages, &tool_calls, mappings);
    (result, serde_json::to_value(&messages).unwrap())
}

#[rstest]
#[case::replaces_the_whole_tool_call(
    json(r#"[{"role": "assistant", "content": "hi", "tool_calls": [{"id": "1", "function": {"name": "f", "arguments": "{}"}}]}]"#),
    &[json(r#"{"id": "1", "function": {"name": "f", "arguments": "{\"a\":\"[x]\"}"}}"#)],
    &[(0, 0)],
    json(r#"[{"role": "assistant", "content": "hi", "tool_calls": [{"id": "1", "function": {"name": "f", "arguments": "{\"a\":\"[x]\"}"}}]}]"#),
)]
#[case::second_tool_call_of_a_later_message(
    json(r#"[{"role": "user"}, {"tool_calls": [{"id": "1"}, {"id": "2"}]}]"#),
    &[json(r#"{"id": "2", "type": "function"}"#)],
    &[(1, 1)],
    json(r#"[{"role": "user"}, {"tool_calls": [{"id": "1"}, {"id": "2", "type": "function"}]}]"#),
)]
#[case::mappings_pick_their_own_tool_call(
    json(r#"[{"tool_calls": [{"id": "a"}]}, {"tool_calls": [{"id": "b"}]}]"#),
    &[json(r#"{"id": "A"}"#), json(r#"{"id": "B"}"#)],
    &[(0, 0), (1, 0)],
    json(r#"[{"tool_calls": [{"id": "A"}]}, {"tool_calls": [{"id": "B"}]}]"#),
)]
#[case::fewer_tool_calls_than_mappings(
    json(r#"[{"tool_calls": [{"id": "a"}, {"id": "b"}]}]"#),
    &[json(r#"{"id": "A"}"#)],
    &[(0, 0), (0, 1)],
    json(r#"[{"tool_calls": [{"id": "A"}, {"id": "b"}]}]"#),
)]
#[case::tool_call_index_past_the_list_is_skipped(
    json(r#"[{"tool_calls": [{"id": "a"}]}]"#),
    &[json(r#"{"id": "A"}"#)],
    &[(0, 1)],
    json(r#"[{"tool_calls": [{"id": "a"}]}]"#),
)]
#[case::null_tool_calls_are_skipped(
    json(r#"[{"content": "x", "tool_calls": null}]"#),
    &[json(r#"{"id": "A"}"#)],
    &[(0, 0)],
    json(r#"[{"content": "x", "tool_calls": null}]"#),
)]
#[case::absent_tool_calls_are_not_created(
    json(r#"[{"content": "x"}]"#),
    &[json(r#"{"id": "A"}"#)],
    &[(0, 0)],
    json(r#"[{"content": "x"}]"#),
)]
fn a_guardrailed_tool_call_replaces_the_one_it_came_from(
    #[case] messages: JsonValue,
    #[case] tool_calls: &[JsonValue],
    #[case] mappings: &[(usize, usize)],
    #[case] expected: JsonValue,
) {
    let (result, rewritten) = rewrite(messages, tool_calls, mappings);
    assert_eq!(result, Ok(()));
    assert_eq!(rewritten, expected);
}

#[test]
fn a_mapping_past_the_last_message_is_an_error() {
    let (result, _) = rewrite(
        json(r#"[{"tool_calls": []}]"#),
        &[json(r#"{"id": "A"}"#)],
        &[(1, 0)],
    );
    assert_eq!(result, Err(ApplyError::MissingEntry { index: 1 }));
}
