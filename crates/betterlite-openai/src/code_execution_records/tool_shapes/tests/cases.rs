#[cfg(test)]
use rstest::rstest;
#[cfg(test)]
use serde_json::json;

#[cfg(test)]
use super::test_object;

#[cfg(test)]
#[test]
fn function_parameters_declares_the_code_property_as_required() {
    let parameters = function_parameters();
    assert_eq!(parameters.kind, "object");
    assert_eq!(parameters.required, vec!["code".to_owned()]);
    assert_eq!(
        parameters.properties["code"].get("type"),
        Some(&"string".to_owned())
    );
}

#[cfg(test)]
fn expected_parameters() -> Value {
    test_object([
        ("type", json!("object")),
        (
            "properties",
            test_object([("code", test_object([("type", json!("string"))]))]),
        ),
        ("required", Value::Array(vec![json!("code")])),
    ])
}

#[cfg(test)]
#[test]
fn function_tool_builds_the_responses_shape() {
    let tool = function_tool(CallDialect::Responses);
    assert_eq!(
        serde_json::to_value(&tool).unwrap(),
        test_object([
            ("type", json!("function")),
            ("name", json!(CODE_EXECUTION_TOOL_NAME)),
            (
                "description",
                json!("Execute python code in a sandbox and return stdout."),
            ),
            ("parameters", expected_parameters()),
        ])
    );
    assert!(matches!(tool, CodeExecutionTool::Responses(_)));
}

#[cfg(test)]
#[test]
fn function_tool_builds_the_chat_completion_shape_nested_under_function() {
    let tool = function_tool(CallDialect::ChatCompletion);
    assert_eq!(
        serde_json::to_value(&tool).unwrap(),
        test_object([
            ("type", json!("function")),
            (
                "function",
                test_object([
                    ("name", json!(CODE_EXECUTION_TOOL_NAME)),
                    (
                        "description",
                        json!("Execute python code in a sandbox and return stdout."),
                    ),
                    ("parameters", expected_parameters()),
                ]),
            ),
        ])
    );
    assert!(matches!(tool, CodeExecutionTool::ChatCompletion(_)));
}

#[cfg(test)]
#[test]
fn function_tool_choice_builds_the_responses_shape() {
    let choice = function_tool_choice(CallDialect::Responses);
    assert_eq!(
        serde_json::to_value(&choice).unwrap(),
        test_object([
            ("type", json!("function")),
            ("name", json!(CODE_EXECUTION_TOOL_NAME)),
        ])
    );
    assert!(matches!(choice, CodeExecutionToolChoice::Responses(_)));
}

#[cfg(test)]
#[test]
fn function_tool_choice_builds_the_chat_completion_shape_nested_under_function() {
    let choice = function_tool_choice(CallDialect::ChatCompletion);
    assert_eq!(
        serde_json::to_value(&choice).unwrap(),
        test_object([
            ("type", json!("function")),
            (
                "function",
                test_object([("name", json!(CODE_EXECUTION_TOOL_NAME))]),
            ),
        ])
    );
}

#[cfg(test)]
#[rstest]
#[case::type_field(test_object([("type", json!("code_interpreter"))]), true)]
#[case::name_field_alias(test_object([("name", json!("code_interpreter"))]), true)]
#[case::name_field_litellm_name(test_object([("name", json!("litellm_code_execution"))]), true)]
#[case::nested_function_name(
    test_object([("function", test_object([("name", json!("litellm_code_execution"))]))]),
    true
)]
#[case::unrelated_tool(test_object([("type", json!("auto"))]), false)]
#[case::not_an_object(json!("auto"), false)]
fn tool_choice_detection_matches_code_interpreter_targets(
    #[case] tool_choice: Value,
    #[case] expected: bool,
) {
    assert_eq!(tool_choice_detection(&tool_choice), expected);
}
