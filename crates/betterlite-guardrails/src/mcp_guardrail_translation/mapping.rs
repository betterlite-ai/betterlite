use super::McpGuardrailHandler;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallType {
    CallMcpTool,
    ListMcpTools,
}

#[must_use]
pub fn guardrail_translation_mappings(call_type: CallType) -> Option<McpGuardrailHandler> {
    match call_type {
        CallType::CallMcpTool | CallType::ListMcpTools => Some(McpGuardrailHandler),
    }
}
