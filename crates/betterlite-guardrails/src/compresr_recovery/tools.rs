use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::response_tool_calls::normalized::get_tool_calls_from_response;
use betterlite_prompts::response_tool_calls::projections::ToolCallResponse;
use betterlite_prompts::response_tool_calls::tools::has_tool_with_name;
use serde::{Deserialize, Serialize};

use super::error::RecoveryError;
use crate::generic_guardrail_contracts::GuardrailToolParam;

pub const COMPRESR_RETRIEVE_TOOL_NAME: &str = "compresr_retrieve";
pub const TOOL_CALLS_KEY: &str = "tool_calls";

// LiteLLM's _build_compresr_retrieve_tool
const RETRIEVE_TOOL_DEFINITION: &str = r#"{
    "type": "function",
    "function": {
        "name": "compresr_retrieve",
        "description": "Retrieve the original, uncompressed content behind a Compresr compression marker. Call this when a compression marker's hash points at content you need in full.",
        "parameters": {
            "type": "object",
            "properties": {
                "hash": {"type": "string", "description": "The 24-character hex hash from the compression marker."}
            },
            "required": ["hash"]
        }
    }
}"#;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CallKind {
    #[default]
    Function,
}

// The gate's record of one compresr_retrieve call, as LiteLLM hands it to the plan builder
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetrieveCall {
    pub id: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: CallKind,
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Object,
}

#[must_use]
pub fn retrieve_calls(response: &ToolCallResponse) -> Vec<RetrieveCall> {
    get_tool_calls_from_response(response, false)
        .into_iter()
        .filter(|call| call.name.as_deref() == Some(COMPRESR_RETRIEVE_TOOL_NAME))
        .map(|call| RetrieveCall {
            id: call.id,
            kind: CallKind::Function,
            name: call.name,
            arguments: call.arguments,
        })
        .collect()
}

pub fn holds_retrieve_tool<Tool>(tools: &[Tool]) -> bool
where
    Tool: Serialize,
{
    let Ok(encoded) = serde_json::to_value(tools) else {
        return false;
    };
    let Ok(rows) = serde_json::from_value::<Vec<Json>>(encoded) else {
        return false;
    };
    has_tool_with_name(&rows, COMPRESR_RETRIEVE_TOOL_NAME)
}

// The request's tools plus the retrieve tool, appended once
pub fn merged_tools(
    existing: Option<&[GuardrailToolParam]>,
) -> Result<Vec<GuardrailToolParam>, RecoveryError> {
    let mut tools = existing.map_or_else(Vec::new, <[_]>::to_vec);
    if !holds_retrieve_tool(&tools) {
        tools.push(serde_json::from_str(RETRIEVE_TOOL_DEFINITION)?);
    }
    Ok(tools)
}
