#![cfg(test)]
use super::{Guardrail, GuardrailInput, GuardrailOutput, McpGuardrailHandler};
use serde_json::json;
use std::future::Future;
struct Echo;
impl Guardrail for Echo {
    fn apply<'a>(
        &'a self,
        input: GuardrailInput,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<GuardrailOutput, String>> + Send + 'a>> {
        Box::pin(async move {
            Ok(GuardrailOutput {
                texts: Some(input.texts),
            })
        })
    }
}
#[tokio::test]
async fn projects_tool_and_argument_texts() {
    let mut data = json!({"mcp_tool_name":"tool","mcp_arguments":{"value":"text"}})
        .as_object()
        .cloned()
        .expect("object");
    McpGuardrailHandler
        .process_input_messages(&mut data, &Echo)
        .await
        .expect("input");
    assert_eq!(data["mcp_arguments"]["value"], "text");
}
