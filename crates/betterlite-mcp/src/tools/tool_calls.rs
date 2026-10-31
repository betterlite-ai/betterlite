use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::pagination::McpSession;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct OpenAiToolCall {
    pub function: OpenAiFunctionCall,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct OpenAiFunctionCall {
    pub name: String,
    #[serde(default)]
    pub arguments: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct McpCallToolRequest {
    pub name: String,
    pub arguments: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CallToolResult {
    pub content: Value,
}

pub fn transform_openai_tool_call_request_to_mcp_tool_call_request(
    call: &OpenAiToolCall,
) -> McpCallToolRequest {
    let arguments = match &call.function.arguments {
        Value::String(text) => serde_json::from_str::<Value>(text)
            .ok()
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default(),
        Value::Object(object) => object.clone(),
        _ => Map::new(),
    };
    McpCallToolRequest {
        name: call.function.name.clone(),
        arguments,
    }
}

pub async fn call_mcp_tool<S: McpSession>(
    session: &mut S,
    request: McpCallToolRequest,
) -> Result<CallToolResult, String> {
    session.call_tool(request).await
}

pub async fn call_openai_tool<S: McpSession>(
    session: &mut S,
    call: &OpenAiToolCall,
) -> Result<CallToolResult, String> {
    let request = transform_openai_tool_call_request_to_mcp_tool_call_request(call);
    call_mcp_tool(session, request).await
}

#[cfg(test)]
mod tool_call_behavior_tests {
    use super::*;

    #[test]
    fn tool_call_behavior_tests_accepts_json_and_invalid_arguments() {
        let call = OpenAiToolCall {
            function: OpenAiFunctionCall {
                name: "echo".to_owned(),
                arguments: Value::String("{\"x\":1}".to_owned()),
            },
        };
        assert_eq!(
            transform_openai_tool_call_request_to_mcp_tool_call_request(&call).arguments["x"],
            1
        );
        let invalid = OpenAiToolCall {
            function: OpenAiFunctionCall {
                name: "echo".to_owned(),
                arguments: Value::String("bad".to_owned()),
            },
        };
        assert!(
            transform_openai_tool_call_request_to_mcp_tool_call_request(&invalid)
                .arguments
                .is_empty()
        );
    }
}
