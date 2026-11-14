use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::response_tool_calls::tools::has_tool_with_name;

use super::HeadroomError;
use crate::generic_guardrail_contracts::GuardrailToolParam;
use crate::headroom_settings::HEADROOM_RETRIEVE_TOOL_NAME;

// LiteLLM's _build_headroom_retrieve_tool, named HEADROOM_RETRIEVE_TOOL_NAME
const TOOL_DEFINITION: &str = r#"{
    "type": "function",
    "function": {
        "name": "headroom_retrieve",
        "description": "Retrieve original content that was compressed by Headroom. Call this when you encounter a compression marker containing a hash.",
        "parameters": {
            "type": "object",
            "properties": {
                "hash": {"type": "string", "description": "The hex hash from the compression marker."},
                "query": {"type": "string", "description": "Optional search query for BM25-ranked retrieval."}
            },
            "required": ["hash"]
        }
    }
}"#;

fn has_retrieve_tool(tools: &[GuardrailToolParam]) -> bool {
    let Ok(encoded) = serde_json::to_value(tools) else {
        return false;
    };
    let Ok(rows) = serde_json::from_value::<Vec<Json>>(encoded) else {
        return false;
    };
    has_tool_with_name(&rows, HEADROOM_RETRIEVE_TOOL_NAME)
}

pub(crate) fn merged(
    existing: Option<&[GuardrailToolParam]>,
) -> Result<Vec<GuardrailToolParam>, HeadroomError> {
    let mut tools = existing.map_or_else(Vec::new, <[_]>::to_vec);
    if !has_retrieve_tool(&tools) {
        tools.push(serde_json::from_str(TOOL_DEFINITION)?);
    }
    Ok(tools)
}
