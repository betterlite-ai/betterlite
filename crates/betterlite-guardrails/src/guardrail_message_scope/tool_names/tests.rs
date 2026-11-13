#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use super::{anthropic_tool_names, openai_tool_name};

fn json(source: &str) -> GuardrailValue {
    serde_json::from_str(source).unwrap()
}

#[rstest]
#[case::function_name(
    r#"{"type":"function","function":{"name":"get_weather"}}"#,
    Some("get_weather")
)]
#[case::flat_name(r#"{"name":"lookup"}"#, Some("lookup"))]
#[case::function_wins_over_flat_name(
    r#"{"name":"flat","function":{"name":"nested"}}"#,
    Some("nested")
)]
#[case::function_without_name_does_not_fall_back(r#"{"name":"flat","function":{}}"#, None)]
#[case::function_not_an_object_uses_flat_name(r#"{"name":"flat","function":"x"}"#, Some("flat"))]
#[case::null_function_uses_flat_name(r#"{"name":"flat","function":null}"#, Some("flat"))]
#[case::name_is_not_text(r#"{"name":4}"#, None)]
#[case::null_name(r#"{"name":null}"#, None)]
#[case::empty_name_is_kept(r#"{"name":""}"#, Some(""))]
#[case::no_name(r#"{"type":"web_search"}"#, None)]
#[case::tool_is_not_an_object(r#""get_weather""#, None)]
fn an_openai_tool_has_one_name(#[case] source: &str, #[case] expected: Option<&str>) {
    let tool = json(source);
    assert_eq!(openai_tool_name(&tool), expected);
}

#[rstest]
#[case::flat_name(r#"{"name":"lookup","input_schema":{}}"#, vec!["lookup"])]
#[case::function_tool_has_both_names(r#"{"type":"function","name":"flat","function":{"name":"nested"}}"#, vec!["flat", "nested"])]
#[case::function_tool_nested_only(r#"{"type":"function","function":{"name":"nested"}}"#, vec!["nested"])]
#[case::same_name_twice_is_kept_twice(r#"{"type":"function","name":"a","function":{"name":"a"}}"#, vec!["a", "a"])]
#[case::nested_name_needs_the_function_type(r#"{"name":"flat","function":{"name":"nested"}}"#, vec!["flat"])]
#[case::other_type_ignores_the_function(r#"{"type":"custom","name":"flat","function":{"name":"nested"}}"#, vec!["flat"])]
#[case::empty_names_are_dropped(r#"{"type":"function","name":"","function":{"name":""}}"#, Vec::<&str>::new())]
#[case::names_that_are_not_text_are_dropped(r#"{"type":"function","name":5,"function":{"name":["x"]}}"#, Vec::<&str>::new())]
#[case::function_not_an_object(r#"{"type":"function","name":"flat","function":"nested"}"#, vec!["flat"])]
#[case::no_names(r#"{"type":"function"}"#, Vec::<&str>::new())]
#[case::tool_is_not_an_object(r#"["a"]"#, Vec::<&str>::new())]
fn an_anthropic_tool_can_have_two_names(#[case] source: &str, #[case] expected: Vec<&str>) {
    let tool = json(source);
    assert_eq!(anthropic_tool_names(&tool), expected);
}
