#![cfg(test)]

use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;

use super::extract_request_tool_names;
use crate::guardrails::chat_guardrail_translation::extraction::fixtures::{json, object};

#[rstest]
#[case::tools_then_functions(
    json(r#"{
        "tools": [{"type": "function", "function": {"name": "b"}}, {"type": "function", "function": {"name": "a"}}],
        "functions": [{"name": "c"}]
    }"#),
    &["b", "a", "c"],
)]
#[case::functions_only(json(r#"{"functions": [{"name": "legacy"}]}"#), &["legacy"])]
#[case::null_lists(json(r#"{"tools": null, "functions": null}"#), &[])]
#[case::missing_lists(json("{}"), &[])]
#[case::tool_type_must_be_function(
    json(r#"{"tools": [{"type": "retrieval", "function": {"name": "x"}}, {"function": {"name": "y"}}]}"#),
    &[],
)]
#[case::empty_or_absent_names_are_skipped(
    json(r#"{
        "tools": [{"type": "function", "function": {"name": ""}}, {"type": "function", "function": {}}, {"type": "function"}],
        "functions": [{"name": ""}, {}]
    }"#),
    &[],
)]
#[case::non_object_entries_are_skipped(
    json(r#"{"tools": ["x", 1, null], "functions": ["y", [1]]}"#),
    &[],
)]
#[case::a_tool_name_outside_function_is_ignored(
    json(r#"{"tools": [{"type": "function", "name": "flat"}]}"#),
    &[],
)]
fn tool_names_come_from_tools_then_functions(
    #[case] request: JsonValue,
    #[case] expected: &[&str],
) {
    assert_eq!(extract_request_tool_names(&object(request)), expected);
}
