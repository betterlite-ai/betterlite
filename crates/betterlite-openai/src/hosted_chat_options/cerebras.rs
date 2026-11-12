use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const SUPPORTED_PARAMS: &[&str] = &[
    "max_tokens",
    "max_completion_tokens",
    "response_format",
    "seed",
    "stop",
    "stream",
    "temperature",
    "top_p",
    "tool_choice",
    "tools",
    "user",
    "max_retries",
    "extra_headers",
];

// B668's class snapshot: every constructor field starts absent and only a supplied non-null value is kept
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct CerebrasSettings {
    pub max_tokens: Option<Value>,
    pub response_format: Option<Value>,
    pub seed: Option<Value>,
    pub stop: Option<Value>,
    pub stream: Option<Value>,
    pub temperature: Option<Value>,
    pub top_p: Option<Value>,
    pub tool_choice: Option<Value>,
    pub tools: Option<Value>,
    pub user: Option<Value>,
    pub reasoning_effort: Option<Value>,
}

// the pinned list appends reasoning_effort only when the model's capability says it supports reasoning
#[must_use]
pub fn supported_params(supports_reasoning: bool) -> Vec<&'static str> {
    let mut params = SUPPORTED_PARAMS.to_vec();
    if supports_reasoning {
        params.push("reasoning_effort");
    }
    params
}
