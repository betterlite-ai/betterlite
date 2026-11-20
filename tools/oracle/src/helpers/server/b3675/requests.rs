use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    Extract,
    ImageSources,
    TopLevelSystem,
    StructuredMessages,
    OutputApply,
    PostCallHook,
}

#[derive(Deserialize)]
pub(crate) struct Request {
    pub(crate) function: Function,
    pub(crate) scenarios: Vec<Scenario>,
}

// A bool on the wire; the newtype keeps struct_excessive_bools off the case input.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(transparent)]
pub(crate) struct CaseFlag(pub(crate) bool);

#[derive(Default, Deserialize)]
#[serde(default)]
pub(crate) struct Flags {
    pub(crate) skip_system_message: CaseFlag,
    pub(crate) skip_tool_message: CaseFlag,
    pub(crate) scan_only_tool_results: CaseFlag,
}

#[derive(Default, Deserialize)]
#[serde(default)]
pub(crate) struct Scenario {
    pub(crate) message: Value,
    pub(crate) msg_idx: usize,
    pub(crate) flags: Flags,
    pub(crate) block: Value,
    pub(crate) hoisted: Value,
    pub(crate) data: Map<String, Value>,
    pub(crate) content: Vec<Value>,
    pub(crate) responses: Vec<String>,
    pub(crate) mappings: Vec<usize>,
    pub(crate) response: Value,
}
