use serde_json::{Map, Value};

use super::CodeExecutionToolCall;
use super::helpers::narrow_tool_call;

#[must_use]
pub fn agentic_tool_calls(tools: &Map<String, Value>) -> Vec<CodeExecutionToolCall> {
    let Some(tool_calls) = tools.get("tool_calls").and_then(Value::as_array) else {
        return Vec::new();
    };
    tool_calls
        .iter()
        .filter_map(Value::as_object)
        .map(narrow_tool_call)
        .collect()
}

include!("tool_calls/tests/cases.rs");
