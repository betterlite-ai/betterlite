use betterlite_openai::api::responses_event_records::event_types::RequiredNullable;
use serde::{Deserialize, Serialize};

// LiteLLM's TypedDict requires every key, so the mirror refuses a missing key while null still reads as None
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "RawMcpToolResult")]
pub struct McpToolResult {
    pub tool_call_id: Option<String>,
    pub result: String,
    pub name: Option<String>,
}

#[derive(Deserialize)]
struct RawMcpToolResult {
    tool_call_id: RequiredNullable<String>,
    result: String,
    name: RequiredNullable<String>,
}

impl From<RawMcpToolResult> for McpToolResult {
    fn from(raw: RawMcpToolResult) -> Self {
        Self {
            tool_call_id: match raw.tool_call_id {
                RequiredNullable::Null => None,
                RequiredNullable::Value(id) => Some(id),
            },
            result: raw.result,
            name: match raw.name {
                RequiredNullable::Null => None,
                RequiredNullable::Value(name) => Some(name),
            },
        }
    }
}

mod tests;
