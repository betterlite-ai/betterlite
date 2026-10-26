#![cfg(test)]

use std::future::Future;

use serde_json::json;

use super::{CallType, Guardrail, GuardrailInput, GuardrailOutput, McpGuardrailHandler};

struct Mask;

impl Guardrail for Mask {
    fn apply<'a>(
        &'a self,
        input: GuardrailInput,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<GuardrailOutput, String>> + Send + 'a>> {
        Box::pin(async move {
            Ok(GuardrailOutput {
                texts: Some(
                    input
                        .texts
                        .into_iter()
                        .map(|text| text.replace("secret", "masked"))
                        .collect(),
                ),
            })
        })
    }
}

#[tokio::test]
async fn process_input_projects_and_writes_back_leaves() {
    let mut data = json!({
        "mcp_tool_name": "lookup",
        "mcp_tool_description": "secret tool",
        "mcp_input_schema": {"description": "secret schema"},
        "mcp_arguments": {"query": "secret", "limit": 2}
    })
    .as_object()
    .cloned()
    .expect("object");
    McpGuardrailHandler
        .process_input_messages(&mut data, &Mask)
        .await
        .expect("translation");
    assert_eq!(data["mcp_tool_description"], "masked tool");
    assert_eq!(data["mcp_input_schema"]["description"], "masked schema");
    assert_eq!(data["mcp_arguments"]["query"], "masked");
    assert_eq!(data["mcp_arguments"]["limit"], 2);
}

#[tokio::test]
async fn process_output_mutates_content_and_structured_content() {
    let mut result = json!({
        "content": [{"type": "text", "text": "secret"}],
        "structuredContent": {"answer": "secret"}
    });
    McpGuardrailHandler
        .process_output_response(&mut result, &Mask)
        .await
        .expect("translation");
    assert_eq!(result["content"][0]["text"], "masked");
    assert_eq!(result["structuredContent"]["answer"], "masked");
}

#[test]
fn mapping_registers_both_mcp_operations() {
    assert!(super::guardrail_translation_mappings(CallType::CallMcpTool).is_some());
    assert!(super::guardrail_translation_mappings(CallType::ListMcpTools).is_some());
}
