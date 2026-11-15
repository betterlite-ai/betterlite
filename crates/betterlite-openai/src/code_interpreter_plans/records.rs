use serde::Serialize;
use serde_json::Value;
use serde_json::value::to_raw_value;
use uuid::Uuid;

use betterlite_context::agentic_loop_plan::FollowupMessage;

use crate::code_execution_records::CodeExecutionToolCall;

pub const CODE_INTERPRETER_CALL_TYPE: &str = "code_interpreter_call";
pub const COMPLETED_STATUS: &str = "completed";

// LiteLLM reports every executed call as a response item of this shape
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CodeInterpreterCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub status: String,
    pub code: String,
    pub container_id: Option<String>,
    pub outputs: Vec<CodeInterpreterOutput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CodeInterpreterOutput {
    Logs { logs: String },
}

impl CodeInterpreterCall {
    #[must_use]
    pub fn completed(code: String, container_id: Option<String>, stdout: String) -> Self {
        let id = Uuid::new_v4().simple();
        let outputs = if stdout.is_empty() {
            Vec::new()
        } else {
            vec![CodeInterpreterOutput::Logs { logs: stdout }]
        };
        Self {
            id: format!("ci_{id}"),
            kind: CODE_INTERPRETER_CALL_TYPE.to_owned(),
            status: COMPLETED_STATUS.to_owned(),
            code,
            container_id,
            outputs,
        }
    }
}

// The gate hands the plan builders the tool calls it detected, shaped by B4120's dialect extractors
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeExecutionToolCalls {
    pub tool_calls: Vec<CodeExecutionToolCall>,
}

pub fn followup_messages(items: &[Value]) -> Result<Vec<FollowupMessage>, serde_json::Error> {
    let mut messages = Vec::with_capacity(items.len());
    for item in items {
        messages.push(FollowupMessage(to_raw_value(item)?));
    }
    Ok(messages)
}
