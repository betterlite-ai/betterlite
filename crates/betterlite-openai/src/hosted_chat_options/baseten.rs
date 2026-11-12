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
    "presence_penalty",
    "frequency_penalty",
    "stream_options",
];
// B668's class snapshot: every constructor field starts absent and only a supplied non-null value is kept
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct BasetenSettings {
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
    pub presence_penalty: Option<Value>,
    pub frequency_penalty: Option<Value>,
    pub stream_options: Option<Value>,
}
