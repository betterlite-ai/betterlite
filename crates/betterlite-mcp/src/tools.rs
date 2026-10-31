mod pagination;
mod schemas;
mod tool_calls;

pub use pagination::{
    McpSession, McpToolOrOpenAi, Page, ToolFormat, list_tools_with_pagination, load_mcp_tools,
};
pub use schemas::{
    AnthropicTool, McpTool, OpenAiFunction, OpenAiTool, ResponsesTool, mcp_input_schema_normalized,
    mcp_tool_to_anthropic, mcp_tool_to_openai, mcp_tool_to_responses_api,
};
pub use tool_calls::{
    CallToolResult, McpCallToolRequest, OpenAiToolCall, call_mcp_tool, call_openai_tool,
    transform_openai_tool_call_request_to_mcp_tool_call_request,
};

#[cfg(test)]
mod mcp_tool_helpers_behavior_tests {
    use super::{McpTool, mcp_input_schema_normalized, mcp_tool_to_openai};
    use serde_json::json;

    #[test]
    fn converters_normalize_partial_schema() {
        let tool = McpTool {
            name: "echo".to_owned(),
            description: None,
            input_schema: json!({"properties":{"x":{"type":"string"}}}),
        };
        assert_eq!(mcp_tool_to_openai(&tool).function.description, "");
        assert_eq!(
            mcp_input_schema_normalized(&tool.input_schema)["type"],
            "object"
        );
    }
}
