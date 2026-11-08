#![cfg(test)]

use super::*;

fn parameters(text: &str) -> MetadataMap {
    serde_json::from_str(text).expect("model parameters")
}

fn tool(name: &str, description: Option<&str>, parameters_json: Option<&str>) -> ToolDefinition {
    ToolDefinition {
        name: name.to_owned(),
        description: description.map(str::to_owned),
        parameters_json: parameters_json.map(str::to_owned),
    }
}

#[rstest::rstest]
#[case::function_wrapper(
    r#"{"tools":[{"type":"function","function":{"name":"get_weather","description":"Weather","parameters":{"type":"object"}}}]}"#
)]
#[case::bare_entry(
    r#"{"tools":[{"name":"get_weather","description":"Weather","parameters":{"type":"object"}}]}"#
)]
fn reads_the_wrapper_and_bare_shapes(#[case] model_parameters: &str) {
    assert_eq!(
        extract_tools(&parameters(model_parameters)),
        vec![tool(
            "get_weather",
            Some("Weather"),
            Some(r#"{"type":"object"}"#)
        )]
    );
}

#[test]
fn falls_back_to_functions_when_tools_is_not_a_list() {
    let model_parameters =
        parameters(r#"{"tools":"none","functions":[{"name":"legacy","parameters":{}}]}"#);
    assert_eq!(
        extract_tools(&model_parameters),
        vec![tool("legacy", None, Some("{}"))]
    );
}

#[test]
fn an_empty_tools_list_wins_over_functions() {
    let model_parameters = parameters(r#"{"tools":[],"functions":[{"name":"legacy"}]}"#);
    assert!(extract_tools(&model_parameters).is_empty());
}

#[rstest::rstest]
#[case::not_an_object(r#"{"tools":["tool"]}"#)]
#[case::null_function(r#"{"tools":[{"function":null,"name":"kept-out"}]}"#)]
#[case::empty_name(r#"{"tools":[{"function":{"name":""}}]}"#)]
#[case::missing_name(r#"{"tools":[{"function":{"description":"no name"}}]}"#)]
#[case::non_object_function(r#"{"tools":[{"function":"name"}]}"#)]
fn skips_unusable_entries(#[case] model_parameters: &str) {
    assert!(extract_tools(&parameters(model_parameters)).is_empty());
}

#[rstest::rstest]
#[case::absent(r#"{"tools":[{"name":"tool"}]}"#)]
#[case::null(r#"{"tools":[{"name":"tool","parameters":null}]}"#)]
fn leaves_parameters_json_none(#[case] model_parameters: &str) {
    assert_eq!(
        extract_tools(&parameters(model_parameters)),
        vec![tool("tool", None, None)]
    );
}

#[test]
fn string_parameters_are_json_encoded() {
    assert_eq!(
        extract_tools(&parameters(
            r#"{"tools":[{"name":"tool","parameters":"raw"}]}"#
        )),
        vec![tool("tool", None, Some(r#""raw""#))]
    );
}
