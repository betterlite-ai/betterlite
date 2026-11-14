use serde_json::{Map, Value};

use crate::code_execution_records::CodeExecutionToolCall;

// The arguments LiteLLM's plan builders take for the call that asked for code execution
pub struct PlanRequest<'request> {
    pub model: &'request str,
    pub messages: &'request Value,
    pub optional_params: &'request Map<String, Value>,
    pub tool_calls: &'request [CodeExecutionToolCall],
    pub kwargs: &'request Map<String, Value>,
}
