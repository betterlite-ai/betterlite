use serde::{Deserialize, Serialize};

// LiteLLM builds every record with all three keys, so a missing key or a null result is refused, never defaulted
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpToolResult {
    pub tool_call_id: Option<String>,
    pub result: String,
    pub name: Option<String>,
}

mod tests;
