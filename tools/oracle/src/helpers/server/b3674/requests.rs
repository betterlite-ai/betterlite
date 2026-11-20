use serde::Deserialize;
use serde_json::{Map, Value};

use super::targets::{InputTargetRequest, MessageTargetRequest, ToolUseTargetRequest};

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Request {
    SystemBlock {
        scenarios: Vec<SystemBlockScenario>,
    },
    MessageText {
        scenarios: Vec<MessageTextScenario>,
    },
    ResponsesToInput {
        scenarios: Vec<ResponsesScenario>,
    },
    ToolUse {
        scenarios: Vec<ToolUseScenario>,
    },
    ToolCalls {
        scenarios: Vec<ToolCallsScenario>,
    },
    Hoisted {
        scenarios: Vec<HoistedScenario>,
    },
    SystemConversion {
        scenarios: Vec<SystemConversionScenario>,
    },
    Fold {
        scenarios: Vec<FoldScenario>,
    },
    Defer {
        scenarios: Vec<DeferScenario>,
    },
    Structured {
        scenarios: Vec<StructuredScenario>,
    },
}

#[derive(Deserialize)]
pub(crate) struct SystemBlockScenario {
    pub(crate) system: Value,
    pub(crate) block_idx: usize,
    pub(crate) response: String,
}

#[derive(Deserialize)]
pub(crate) struct MessageTextScenario {
    pub(crate) message: Map<String, Value>,
    pub(crate) target: MessageTargetRequest,
    pub(crate) response: String,
}

#[derive(Deserialize)]
pub(crate) struct ResponsesScenario {
    pub(crate) data: Map<String, Value>,
    pub(crate) responses: Vec<String>,
    pub(crate) scanned: Vec<ScannedTextRequest>,
}

#[derive(Deserialize)]
pub(crate) struct ScannedTextRequest {
    pub(crate) text: String,
    pub(crate) target: InputTargetRequest,
}

#[derive(Deserialize)]
pub(crate) struct ToolUseScenario {
    pub(crate) message: Map<String, Value>,
    pub(crate) target: ToolUseTargetRequest,
    #[serde(default)]
    pub(crate) shape_name: Option<String>,
    pub(crate) arguments: String,
}

#[derive(Deserialize)]
pub(crate) struct ToolCallsScenario {
    pub(crate) messages: Vec<Value>,
    pub(crate) scanned: Vec<ScannedCallRequest>,
    pub(crate) pre: Vec<Value>,
    #[serde(default)]
    pub(crate) returned: Option<Vec<Value>>,
    #[serde(default)]
    pub(crate) guardrail_name: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct ScannedCallRequest {
    pub(crate) tool_call: Value,
    pub(crate) target: ToolUseTargetRequest,
}

#[derive(Deserialize)]
pub(crate) struct HoistedScenario {
    pub(crate) data: Map<String, Value>,
}

#[derive(Deserialize)]
pub(crate) struct SystemConversionScenario {
    pub(crate) message: Value,
}

#[derive(Deserialize)]
pub(crate) struct FoldScenario {
    pub(crate) data: Map<String, Value>,
    pub(crate) leading: Vec<Value>,
    pub(crate) include_existing: bool,
}

#[derive(Deserialize)]
pub(crate) struct DeferScenario {
    pub(crate) messages: Vec<Value>,
}

#[derive(Deserialize)]
pub(crate) struct StructuredScenario {
    pub(crate) data: Map<String, Value>,
    pub(crate) messages: Vec<Value>,
    pub(crate) preserve_system_messages: bool,
    pub(crate) compute_hoisted: bool,
}
