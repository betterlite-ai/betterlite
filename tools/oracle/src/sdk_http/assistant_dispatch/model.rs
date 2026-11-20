use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_server::api::assistant_dispatch::AssistantOperation;
use serde::{Deserialize, Serialize};

use super::super::result::Failure;

// The proxy route's body, typed per field; params carries whatever the payload does not name.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Payload {
    pub assistant_id: Option<String>,
    pub model: Option<String>,
    pub name: Option<String>,
    pub role: Option<String>,
    pub content: Option<Json>,
    pub attachments: Option<Json>,
    pub messages: Option<Json>,
    pub metadata: Option<Json>,
    pub tool_resources: Option<Json>,
    pub instructions: Option<String>,
    pub additional_instructions: Option<String>,
    pub stream: Option<Json>,
    pub tools: Option<Json>,
    #[serde(default)]
    pub params: BTreeMap<String, Json>,
}

// assistants_config as Router receives it, plus whether the proxy startup block builds it from YAML.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigProjection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_llm_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub litellm_params: Option<BTreeMap<String, Json>>,
}

// One call of one installed wrapper: the operation, its body, the provider and path argument.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OperationCall {
    pub operation: AssistantOperation,
    #[serde(default)]
    pub payload: Payload,
    pub provider: Option<String>,
    pub thread_id: Option<String>,
    pub assistant_id: Option<String>,
    pub config: Option<ConfigProjection>,
    #[serde(default)]
    pub startup: bool,
}

// A case runs the installed wrappers under one assistants_config each, in order.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub runs: Vec<OperationCall>,
}

// The ids the wrapper returned, the status a polled run carries, the events a consumed stream yielded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub ids: Vec<String>,
    pub status: Option<String>,
    pub events: Option<Vec<String>>,
    pub failure: Option<Failure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub results: Vec<Step>,
}
