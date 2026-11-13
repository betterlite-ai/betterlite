use rstest::rstest;

use crate::sampling_convert::openai_wire::{OpenAiToolChoice, ToolKind};
use crate::sampling_convert::tools::{
    McpTool, McpToolChoice, ToolChoiceMode, convert_tool_choice, convert_tools,
};

use super::fixtures::object;

fn tool(name: &str, description: &str, schema: &str) -> McpTool {
    McpTool {
        name: name.to_owned(),
        description: description.to_owned(),
        input_schema: object(schema),
    }
}

#[test]
fn no_tools_or_an_empty_list_convert_to_none() {
    assert_eq!(convert_tools(None), None);
    assert_eq!(convert_tools(Some(&[])), None);
}

#[test]
fn a_named_tool_converts_with_its_description_and_schema() {
    let tools = vec![tool(
        "lookup",
        "find things",
        r#"{"type": "object", "properties": {"q": {"type": "string"}}}"#,
    )];
    let converted = convert_tools(Some(&tools)).expect("tools");
    assert_eq!(converted.len(), 1);
    let tool = converted.first().expect("tool");
    assert_eq!(tool.kind, ToolKind::Function);
    assert_eq!(tool.function.name, "lookup");
    assert_eq!(tool.function.description, "find things");
    assert_eq!(
        tool.function.parameters.as_ref(),
        r#"{"type": "object", "properties": {"q": {"type": "string"}}}"#
    );
}

#[test]
fn a_tool_without_a_description_gets_an_empty_one() {
    let tools = vec![tool("lookup", "", r#"{"type": "object"}"#)];
    let converted = convert_tools(Some(&tools)).expect("tools");
    assert_eq!(
        converted.first().expect("tool").function.description,
        String::new()
    );
}

#[test]
fn an_empty_schema_is_forwarded_as_given() {
    let tools = vec![tool("lookup", "", "{}")];
    let converted = convert_tools(Some(&tools)).expect("tools");
    assert_eq!(
        converted
            .first()
            .expect("tool")
            .function
            .parameters
            .as_ref(),
        "{}"
    );
}

#[test]
fn no_tool_choice_converts_to_none() {
    assert_eq!(convert_tool_choice(None), None);
}

#[rstest]
#[case::auto(ToolChoiceMode::Auto, OpenAiToolChoice::Auto)]
#[case::required(ToolChoiceMode::Required, OpenAiToolChoice::Required)]
#[case::none(ToolChoiceMode::None, OpenAiToolChoice::None)]
fn tool_choice_modes_convert(#[case] mode: ToolChoiceMode, #[case] expected: OpenAiToolChoice) {
    let choice = McpToolChoice { mode };
    assert_eq!(convert_tool_choice(Some(&choice)), Some(expected));
}

#[test]
fn a_tool_choice_without_a_mode_defaults_to_auto() {
    assert_eq!(
        convert_tool_choice(Some(&McpToolChoice::default())),
        Some(OpenAiToolChoice::Auto)
    );
}

#[test]
fn tool_documents_decode_from_the_mcp_wire() {
    let tool: McpTool = serde_json::from_str(
        r#"{"name":"lookup","description":"find","inputSchema":{"type":"object"}}"#,
    )
    .expect("tool");
    assert_eq!(tool.name, "lookup");
    assert_eq!(tool.description, "find");
    assert_eq!(tool.input_schema.as_ref(), r#"{"type":"object"}"#);
    let choice: McpToolChoice = serde_json::from_str(r#"{"mode":"required"}"#).expect("choice");
    assert_eq!(choice.mode, ToolChoiceMode::Required);
    let empty: McpToolChoice = serde_json::from_str("{}").expect("choice");
    assert_eq!(empty.mode, ToolChoiceMode::Auto);
}

#[test]
fn null_tool_fields_take_their_defaults() {
    let tool: McpTool =
        serde_json::from_str(r#"{"name":"bare","description":null,"inputSchema":{}}"#)
            .expect("tool");
    assert_eq!(tool.description, String::new());
    let choice: McpToolChoice = serde_json::from_str(r#"{"mode":null}"#).expect("choice");
    assert_eq!(choice.mode, ToolChoiceMode::Auto);
}
