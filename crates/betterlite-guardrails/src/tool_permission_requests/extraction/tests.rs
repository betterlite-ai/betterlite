#![cfg(test)]

use rstest::rstest;

use super::extract_request_tools;
use crate::tool_permission_requests::error::RequestToolsError;
use crate::tool_permission_requests::tests::{functions, request, tools};

#[test]
fn tools_yield_their_function_names_in_order() {
    let extracted =
        extract_request_tools(&request(&[("tools", &tools(&["Bash", "Read"]))])).unwrap();
    assert_eq!(extracted.names(), vec!["Bash", "Read"]);
    assert_eq!(
        extracted.tools,
        Some(vec![Some("Bash".to_owned()), Some("Read".to_owned())])
    );
}

#[rstest]
#[case::non_function_type(r#"{"type": "web_search"}"#)]
#[case::non_function_type_with_a_function_block(r#"{"type": "mcp", "function": {"name": "Bash"}}"#)]
#[case::function_without_a_function_block(r#"{"type": "function"}"#)]
#[case::function_with_a_null_block(r#"{"type": "function", "function": null}"#)]
#[case::function_block_without_a_name(r#"{"type": "function", "function": {"description": "x"}}"#)]
#[case::function_block_with_a_null_name(r#"{"type": "function", "function": {"name": null}}"#)]
#[case::no_type_at_all(r#"{"function": {"name": "Bash"}}"#)]
#[case::null_type(r#"{"type": null, "function": {"name": "Bash"}}"#)]
fn unnamed_or_non_function_tools_are_skipped(#[case] tool: &str) {
    let extracted = extract_request_tools(&request(&[("tools", &format!("[{tool}]"))])).unwrap();
    assert!(extracted.names().is_empty());
    assert_eq!(extracted.tools, Some(vec![None]));
}

#[test]
fn legacy_functions_yield_their_names() {
    let listed = r#"[{"name": "lookup"}, {"description": "unnamed"}, {"name": "send"}]"#;
    let extracted = extract_request_tools(&request(&[("functions", listed)])).unwrap();
    assert_eq!(extracted.names(), vec!["lookup", "send"]);
    assert_eq!(
        extracted.functions,
        Some(vec![
            Some("lookup".to_owned()),
            None,
            Some("send".to_owned())
        ])
    );
}

#[rstest]
#[case::tool_choice_auto("tool_choice", r#""auto""#, None)]
#[case::tool_choice_none("tool_choice", r#""none""#, None)]
#[case::tool_choice_required("tool_choice", r#""required""#, None)]
#[case::tool_choice_null("tool_choice", "null", None)]
#[case::tool_choice_string_name("tool_choice", r#""Bash""#, Some("Bash"))]
#[case::tool_choice_function_object(
    "tool_choice",
    r#"{"type": "function", "function": {"name": "Bash"}}"#,
    Some("Bash")
)]
#[case::tool_choice_non_function_object(
    "tool_choice",
    r#"{"type": "custom", "function": {"name": "Bash"}}"#,
    None
)]
#[case::tool_choice_object_without_a_type("tool_choice", r#"{"function": {"name": "Bash"}}"#, None)]
#[case::tool_choice_object_without_a_function("tool_choice", r#"{"type": "function"}"#, None)]
#[case::tool_choice_object_without_a_name(
    "tool_choice",
    r#"{"type": "function", "function": {}}"#,
    None
)]
#[case::function_call_auto("function_call", r#""auto""#, None)]
#[case::function_call_none("function_call", r#""none""#, None)]
#[case::function_call_required_is_a_name("function_call", r#""required""#, Some("required"))]
#[case::function_call_null("function_call", "null", None)]
#[case::function_call_string_name("function_call", r#""lookup""#, Some("lookup"))]
#[case::function_call_object_name("function_call", r#"{"name": "lookup"}"#, Some("lookup"))]
#[case::function_call_object_without_a_name("function_call", r#"{"arguments": "{}"}"#, None)]
fn a_forced_choice_names_a_tool_only_when_it_forces_one(
    #[case] key: &str,
    #[case] value: &str,
    #[case] expected: Option<&str>,
) {
    let extracted = extract_request_tools(&request(&[(key, value)])).unwrap();
    assert_eq!(extracted.names(), expected.into_iter().collect::<Vec<_>>());
}

#[test]
fn names_come_from_tools_then_functions_then_tool_choice_then_function_call() {
    let body = request(&[
        ("function_call", r#""fourth""#),
        ("tool_choice", r#""third""#),
        ("functions", &functions(&["second"])),
        ("tools", &tools(&["first"])),
    ]);
    assert_eq!(
        extract_request_tools(&body).unwrap().names(),
        vec!["first", "second", "third", "fourth"]
    );
}

#[rstest]
#[case::absent(&[])]
#[case::null_lists(&[("tools", "null"), ("functions", "null"), ("tool_choice", "null"), ("function_call", "null")])]
#[case::empty_lists(&[("tools", "[]"), ("functions", "[]")])]
#[case::only_unforced_choices(&[("tool_choice", r#""auto""#), ("function_call", r#""none""#)])]
fn a_request_without_tools_yields_no_names(#[case] fields: &[(&str, &str)]) {
    assert!(
        extract_request_tools(&request(fields))
            .unwrap()
            .names()
            .is_empty()
    );
}

#[test]
fn an_absent_or_null_list_stays_absent_and_an_empty_list_stays_empty() {
    let absent = extract_request_tools(&request(&[("tools", "null")])).unwrap();
    assert_eq!(absent.tools, None);
    assert_eq!(absent.functions, None);
    let empty = extract_request_tools(&request(&[("tools", "[]"), ("functions", "[]")])).unwrap();
    assert_eq!(empty.tools, Some(vec![]));
    assert_eq!(empty.functions, Some(vec![]));
}

#[rstest]
#[case::tools_not_a_list("tools", r#""Bash""#, "tools", "a list")]
#[case::functions_not_a_list("functions", r#"{"name": "x"}"#, "functions", "a list")]
#[case::tool_not_an_object(
    "tools",
    r#"[{"type": "web_search"}, "Bash"]"#,
    "tools[1]",
    "an object"
)]
#[case::function_not_an_object("functions", "[3]", "functions[0]", "an object")]
#[case::tool_type_not_a_string("tools", r#"[{"type": 1}]"#, "tools[0].type", "a string")]
#[case::function_block_not_an_object(
    "tools",
    r#"[{"type": "function", "function": "x"}]"#,
    "tools[0].function",
    "an object"
)]
#[case::tool_name_not_a_string(
    "tools",
    r#"[{"type": "function", "function": {"name": 4}}]"#,
    "tools[0].function.name",
    "a string"
)]
#[case::function_name_not_a_string(
    "functions",
    r#"[{"name": ["x"]}]"#,
    "functions[0].name",
    "a string"
)]
#[case::tool_choice_not_text_or_object("tool_choice", "7", "tool_choice", "a string or an object")]
#[case::tool_choice_type_not_a_string(
    "tool_choice",
    r#"{"type": 1}"#,
    "tool_choice.type",
    "a string"
)]
#[case::tool_choice_function_not_an_object(
    "tool_choice",
    r#"{"type": "function", "function": "x"}"#,
    "tool_choice.function",
    "an object"
)]
#[case::tool_choice_name_not_a_string(
    "tool_choice",
    r#"{"type": "function", "function": {"name": 1}}"#,
    "tool_choice.function.name",
    "a string"
)]
#[case::function_call_not_text_or_object(
    "function_call",
    r#"["x"]"#,
    "function_call",
    "a string or an object"
)]
#[case::function_call_name_not_a_string(
    "function_call",
    r#"{"name": 1}"#,
    "function_call.name",
    "a string"
)]
fn a_value_of_the_wrong_type_is_an_error_that_names_the_field(
    #[case] key: &str,
    #[case] value: &str,
    #[case] field: &str,
    #[case] expected: &'static str,
) {
    assert_eq!(
        extract_request_tools(&request(&[(key, value)])),
        Err(RequestToolsError::WrongType(field.to_owned(), expected))
    );
}
