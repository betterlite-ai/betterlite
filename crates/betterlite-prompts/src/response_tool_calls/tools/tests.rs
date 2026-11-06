#![cfg(test)]

use super::has_tool_with_name;
use crate::arguments::json::Json;
use rstest::rstest;

fn tools(json: &str) -> Vec<Json> {
    serde_json::from_str(json).expect("tools")
}

#[rstest]
#[case::openai_function_tool_matches(
    r#"[{"type":"function","function":{"name":"alpha"}}]"#,
    "alpha",
    true
)]
#[case::anthropic_shape_matches_top_level_name(
    r#"[{"name":"beta","input_schema":{"type":"object"}}]"#,
    "beta",
    true
)]
#[case::function_type_without_object_function_falls_back_to_name(
    r#"[{"type":"function","function":"alpha","name":"beta"}]"#,
    "beta",
    true
)]
#[case::non_object_tools_are_skipped(r#"["alpha",3,null]"#, "alpha", false)]
#[case::absent_function_name_does_not_match(r#"[{"type":"function","function":{}}]"#, "", false)]
#[case::empty_top_level_name_matches_empty_name(r#"[{"name":""}]"#, "", true)]
#[case::empty_tool_list_has_no_tool("[]", "alpha", false)]
#[case::name_must_match_exactly(
    r#"[{"type":"function","function":{"name":"alpha"}}]"#,
    "alphabet",
    false
)]
#[case::later_tool_can_match(
    r#"[{"type":"function","function":{"name":"alpha"}},{"type":"retrieval","name":"beta"}]"#,
    "beta",
    true
)]
fn response_tool_calls_behavior_tests_behavior(
    #[case] json: &str,
    #[case] tool_name: &str,
    #[case] expected: bool,
) {
    assert_eq!(has_tool_with_name(&tools(json), tool_name), expected);
}
