use serde::Deserialize;
use serde_json::{Map, Value};

use super::targets::{TargetRequest, ToolUseTargetRequest};

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    SystemBlock,
    MessageText,
    ResponsesToInput,
    ToolUse,
    ToolCalls,
    Hoisted,
    SystemConversion,
    Fold,
    Defer,
    Structured,
}

#[derive(Deserialize)]
pub(crate) struct Request {
    pub(crate) function: Function,
    pub(crate) scenarios: Vec<Scenario>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
pub(crate) struct Scenario {
    pub(crate) system: Value,
    pub(crate) block_idx: usize,
    pub(crate) response: String,
    pub(crate) message: Map<String, Value>,
    pub(crate) target: Option<TargetRequest>,
    pub(crate) tool_target: ToolUseTargetRequest,
    pub(crate) shape_name: Option<String>,
    pub(crate) arguments: String,
    pub(crate) data: Map<String, Value>,
    pub(crate) responses: Vec<String>,
    pub(crate) scanned_texts: Vec<ScannedTextRequest>,
    pub(crate) scanned_calls: Vec<ScannedCallRequest>,
    pub(crate) pre: Vec<Value>,
    pub(crate) returned: Option<Vec<Value>>,
    pub(crate) guardrail_name: Option<String>,
    pub(crate) messages: Vec<Value>,
    pub(crate) leading: Vec<Value>,
    pub(crate) include_existing: bool,
    pub(crate) preserve_system_messages: bool,
    pub(crate) compute_hoisted: bool,
}

#[derive(Deserialize)]
pub(crate) struct ScannedTextRequest {
    pub(crate) text: String,
    pub(crate) target: TargetRequest,
}

#[derive(Deserialize)]
pub(crate) struct ScannedCallRequest {
    pub(crate) tool_call: Value,
    pub(crate) tool_target: ToolUseTargetRequest,
}
