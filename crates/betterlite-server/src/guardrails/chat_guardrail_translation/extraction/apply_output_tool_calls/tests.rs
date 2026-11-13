#![cfg(test)]

use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;

use super::apply_output_tool_calls;
use crate::guardrails::chat_guardrail_translation::extraction::apply_error::ApplyError;
use crate::guardrails::chat_guardrail_translation::extraction::fixtures::{json, object};

fn rewrite(
    source: JsonValue,
    guardrailed: &[JsonValue],
    mappings: &[(usize, usize)],
) -> (Result<(), ApplyError>, JsonValue) {
    let mut response = object(source);
    let tool_calls: Vec<_> = guardrailed.iter().cloned().map(object).collect();
    let result = apply_output_tool_calls(&mut response, &tool_calls, mappings);
    (result, serde_json::to_value(&response).unwrap())
}

fn response_with_calls(calls: &JsonValue) -> JsonValue {
    json(&format!(
        r#"{{"choices": [{{"message": {{"content": null, "tool_calls": {calls}}}}}]}}"#
    ))
}

#[rstest]
#[case::arguments_and_name(
    json(r#"[{"id": "1", "type": "function", "function": {"name": "f", "arguments": "{\"a\":1}"}}]"#),
    json(r#"{"id": "9", "type": "other", "function": {"name": "g", "arguments": "{\"a\":\"[x]\"}"}}"#),
    json(r#"[{"id": "1", "type": "function", "function": {"name": "g", "arguments": "{\"a\":\"[x]\"}"}}]"#),
)]
#[case::arguments_only(
    json(r#"[{"id": "1", "function": {"name": "f", "arguments": "{}"}}]"#),
    json(r#"{"function": {"arguments": "{\"b\":2}"}}"#),
    json(r#"[{"id": "1", "function": {"name": "f", "arguments": "{\"b\":2}"}}]"#)
)]
#[case::name_only(
    json(r#"[{"id": "1", "function": {"name": "f", "arguments": "{}"}}]"#),
    json(r#"{"function": {"name": "g"}}"#),
    json(r#"[{"id": "1", "function": {"name": "g", "arguments": "{}"}}]"#)
)]
#[case::other_function_keys_are_not_copied(
    json(r#"[{"id": "1", "function": {"name": "f"}}]"#),
    json(r#"{"function": {"name": "f", "description": "new"}}"#),
    json(r#"[{"id": "1", "function": {"name": "f"}}]"#)
)]
#[case::guardrailed_call_without_a_function(
    json(r#"[{"id": "1", "function": {"name": "f"}}]"#),
    json(r#"{"id": "2"}"#),
    json(r#"[{"id": "1", "function": {"name": "f"}}]"#)
)]
#[case::guardrailed_function_that_is_not_an_object(
    json(r#"[{"id": "1", "function": {"name": "f"}}]"#),
    json(r#"{"function": "g"}"#),
    json(r#"[{"id": "1", "function": {"name": "f"}}]"#)
)]
#[case::existing_call_without_a_function(
    json(r#"[{"id": "1"}]"#),
    json(r#"{"function": {"name": "g"}}"#),
    json(r#"[{"id": "1"}]"#)
)]
fn a_guardrailed_call_updates_only_the_function_name_and_arguments(
    #[case] existing: JsonValue,
    #[case] guardrailed: JsonValue,
    #[case] expected: JsonValue,
) {
    let (result, rewritten) = rewrite(response_with_calls(&existing), &[guardrailed], &[(0, 0)]);
    assert_eq!(result, Ok(()));
    assert_eq!(rewritten, response_with_calls(&expected));
}

#[test]
fn each_mapping_picks_its_choice_and_tool_call() {
    let response = json(
        r#"{"choices": [
        {"message": {"tool_calls": [{"function": {"name": "a"}}]}},
        {"message": {"tool_calls": [{"function": {"name": "b"}}, {"function": {"name": "c"}}]}}
    ]}"#,
    );
    let (result, rewritten) = rewrite(
        response,
        &[
            json(r#"{"function": {"name": "C"}}"#),
            json(r#"{"function": {"name": "A"}}"#),
        ],
        &[(1, 1), (0, 0)],
    );
    assert_eq!(result, Ok(()));
    assert_eq!(
        rewritten,
        json(
            r#"{"choices": [
            {"message": {"tool_calls": [{"function": {"name": "A"}}]}},
            {"message": {"tool_calls": [{"function": {"name": "b"}}, {"function": {"name": "C"}}]}}
        ]}"#
        )
    );
}

#[rstest]
#[case::fewer_tool_calls_than_mappings(response_with_calls(&json(r#"[{"function": {"name": "a"}}]"#)), vec![], vec![(0, 0)])]
#[case::tool_call_index_past_the_list(response_with_calls(&json(r#"[{"function": {"name": "a"}}]"#)), vec![json(r#"{"function": {"name": "A"}}"#)], vec![(0, 1)])]
#[case::null_tool_calls(response_with_calls(&json("null")), vec![json(r#"{"function": {"name": "A"}}"#)], vec![(0, 0)])]
#[case::choice_without_a_message(json(r#"{"choices": [{"delta": {"tool_calls": [{"function": {"name": "a"}}]}}]}"#), vec![json(r#"{"function": {"name": "A"}}"#)], vec![(0, 0)])]
fn a_call_that_has_nowhere_to_go_leaves_the_response_as_it_was(
    #[case] response: JsonValue,
    #[case] tool_calls: Vec<JsonValue>,
    #[case] mappings: Vec<(usize, usize)>,
) {
    let (result, rewritten) = rewrite(response.clone(), &tool_calls, &mappings);
    assert_eq!(result, Ok(()));
    assert_eq!(rewritten, response);
}

#[test]
fn a_mapping_past_the_last_choice_is_an_error() {
    let (result, _) = rewrite(
        response_with_calls(&json("[]")),
        &[json(r#"{"function": {"name": "A"}}"#)],
        &[(1, 0)],
    );
    assert_eq!(result, Err(ApplyError::MissingEntry { index: 1 }));
}
