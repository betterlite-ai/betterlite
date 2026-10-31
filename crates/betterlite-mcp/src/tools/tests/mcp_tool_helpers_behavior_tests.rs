#![cfg(test)]

use betterlite_prompts::arguments::json::Object;
use rstest::rstest;
use std::time::Duration;

use super::{assert_json, fake, timeouts, tool};
use crate::tools::pagination::{Page, ToolFormat, load_mcp_tools};
use crate::tools::schemas::{
    McpTool, mcp_input_schema_normalized, mcp_tool_to_anthropic, mcp_tool_to_openai,
    mcp_tool_to_responses_api,
};

#[rstest]
#[case::absent(r#"{"name":"echo"}"#)]
#[case::null(r#"{"name":"echo","inputSchema":null}"#)]
#[case::empty(r#"{"name":"echo","inputSchema":{}}"#)]
fn converters_normalize_partial_schema(#[case] input: &str) {
    let tool: McpTool = serde_json::from_str(input).unwrap();
    assert_json(
        &mcp_tool_to_openai(&tool),
        r#"{"type":"function","function":{"name":"echo","description":"","parameters":{"type":"object","properties":{},"additionalProperties":false},"strict":false}}"#,
    );
    assert_json(
        &mcp_tool_to_responses_api(&tool),
        r#"{"type":"function","name":"echo","description":"","parameters":{"type":"object","properties":{},"additionalProperties":false},"strict":false}"#,
    );
    assert_json(
        &mcp_tool_to_anthropic(&tool).unwrap(),
        r#"{"type":"custom","name":"echo","description":"","input_schema":{"type":"object","properties":{}}}"#,
    );
}

#[test]
fn explicit_fields_are_preserved_without_mutating_input() {
    let schema: Object = serde_json::from_str(
        r#"{"type":"array","properties":null,"additionalProperties":true,"required":["x"]}"#,
    )
    .unwrap();
    assert_eq!(mcp_input_schema_normalized(Some(&schema)), schema);
    let source: McpTool = serde_json::from_str(r#"{"name":"lookup","description":"Find item","inputSchema":{"properties":{"x":{"type":"string"}}}}"#).unwrap();
    let original = source.clone();
    let chat = mcp_tool_to_openai(&source);
    let responses = mcp_tool_to_responses_api(&source);
    assert_eq!(chat.function.name, "lookup");
    assert_eq!(responses.name, "lookup");
    assert_eq!(chat.function.description, "Find item");
    assert_eq!(responses.description, "Find item");
    assert_json(
        &chat.function.parameters,
        r#"{"type":"object","properties":{"x":{"type":"string"}},"additionalProperties":false}"#,
    );
    assert_eq!(source, original);
}

#[test]
fn anthropic_uses_the_shared_sanitizer() {
    let tool: McpTool = serde_json::from_str(r##"{"name":"echo","description":"Echo","inputSchema":{"title":"drop","type":"array","additionalProperties":true,"strict":true,"required":["x"],"$defs":{},"properties":{"x":{"$ref":"#/definitions/X"}},"definitions":{"X":{"type":"string"}}}}"##).unwrap();
    assert_json(
        &mcp_tool_to_anthropic(&tool).unwrap(),
        r#"{"name":"echo","description":"Echo","type":"custom","input_schema":{"type":"object","properties":{"x":{"type":"string"}},"additionalProperties":true,"required":["x"],"$defs":{},"strict":true}}"#,
    );
}

#[rstest]
#[case::mcp(ToolFormat::Mcp)]
#[case::openai(ToolFormat::Openai)]
#[tokio::test(start_paused = true)]
async fn load_selects_format(#[case] format: ToolFormat) {
    let original = tool();
    let mut session = fake(vec![(
        Duration::ZERO,
        Page {
            tools: vec![original.clone()],
            next_cursor: None,
        },
    )]);
    let result = serde_json::to_value(
        load_mcp_tools(&mut session, format, timeouts(60, 30))
            .await
            .unwrap(),
    )
    .unwrap();
    let expected = match format {
        ToolFormat::Mcp => serde_json::to_value(vec![original]).unwrap(),
        ToolFormat::Openai => serde_json::to_value(vec![mcp_tool_to_openai(&original)]).unwrap(),
    };
    assert_eq!(result, expected);
    assert_eq!(session.cursors, vec![None]);
}

#[test]
fn format_defaults_and_rejects_invalid() {
    assert_eq!(ToolFormat::default(), ToolFormat::Mcp);
    serde_json::from_str::<ToolFormat>(r#""responses""#).unwrap_err();
}
