use serde::{Deserialize, Serialize};

mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpCallType {
    CallMcpTool,
    ListMcpTools,
}

// both call types run process_input::process_input_messages and process_output::process_output_response
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McpGuardrailTranslationHandler;

pub const GUARDRAIL_TRANSLATION_MAPPINGS: [(McpCallType, McpGuardrailTranslationHandler); 2] = [
    (McpCallType::CallMcpTool, McpGuardrailTranslationHandler),
    (McpCallType::ListMcpTools, McpGuardrailTranslationHandler),
];
