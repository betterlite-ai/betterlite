use serde_json::{Map, Value};

use crate::code_execution_records::CodeExecutionToolCall;

// The arguments LiteLLM's plan builders take for the call that asked for code execution
pub struct PlanRequest<'a> {
    pub model: &'a str,
    pub messages: &'a Value,
    pub optional_params: &'a Map<String, Value>,
    pub tool_calls: &'a [CodeExecutionToolCall],
    pub kwargs: &'a Map<String, Value>,
}
