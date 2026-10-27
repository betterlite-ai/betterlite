#![cfg(test)]
use rstest::rstest;

use crate::mcp_guardrail_translation::process_input::process_input_messages;
use crate::mcp_guardrail_translation::tests::{Reply, ScriptedGuardrail, request, texts, to_json};

#[rstest]
#[case::no_name(r#"{"mcp_arguments":{"to":"secret"}}"#)]
#[case::empty_names(r#"{"mcp_tool_name":"","name":"","mcp_arguments":{"to":"secret"}}"#)]
#[tokio::test]
async fn a_missing_tool_name_leaves_the_request_unchanged(#[case] json: &str) {
    let original = request(json);
    let mut call = original.clone();
    let guardrail = ScriptedGuardrail::new(Reply::Mask("secret"));
    process_input_messages(&mut call, &guardrail, None)
        .await
        .unwrap();
    assert!(guardrail.calls().is_empty());
    assert_eq!(call, original);
}
#[rstest]
#[case::mcp_fields(
    r#"{"mcp_tool_name":"a","mcp_tool_description":"d","mcp_arguments":{"q":"v"}}"#,
    "a",
    "d"
)]
#[case::plain_fields(r#"{"name":"b","description":"e","arguments":{"q":"v"}}"#, "b", "e")]
#[case::empty_mcp_fields(r#"{"mcp_tool_name":"","name":"c","mcp_tool_description":"","description":"f","mcp_arguments":{},"arguments":{"q":"v"}}"#, "c", "f")]
fn empty_mcp_fields_fall_back_to_the_plain_ones(
    #[case] json: &str,
    #[case] name: &str,
    #[case] description: &str,
) {
    let call = request(json);
    assert_eq!(call.tool_name(), Some(name));
    assert_eq!(call.tool_description(), Some(description));
    assert_eq!(to_json(&call.tool_arguments()), r#"{"q":"v"}"#);
}
#[rstest]
#[case::no_arguments(r#"{"mcp_tool_name":"t","description":"Tool"}"#)]
#[case::null_arguments(r#"{"mcp_tool_name":"t","description":"Tool","mcp_arguments":null}"#)]
#[case::no_schema(r#"{"mcp_tool_name":"t","description":"Tool","mcp_input_schema":{}}"#)]
#[tokio::test]
async fn missing_arguments_and_schema_project_as_empty(#[case] json: &str) {
    let mut call = request(json);
    let guardrail = ScriptedGuardrail::new(Reply::Mask("secret"));
    process_input_messages(&mut call, &guardrail, None)
        .await
        .unwrap();
    assert_eq!(guardrail.calls()[0].0.texts, texts(&["Tool"]));
    assert_eq!(call.modified_arguments, None);
}
