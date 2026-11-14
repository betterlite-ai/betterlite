use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::response_tool_calls::tools::has_tool_with_name;
use serde_json::json;

use super::HeadroomError;
use crate::generic_guardrail_contracts::GuardrailToolParam;
use crate::headroom_settings::HEADROOM_RETRIEVE_TOOL_NAME;

const TOOL_DESCRIPTION: &str = "Retrieve original content that was compressed by Headroom. Call this when you encounter a compression marker containing a hash.";
const HASH_DESCRIPTION: &str = "The hex hash from the compression marker.";
const QUERY_DESCRIPTION: &str = "Optional search query for BM25-ranked retrieval.";

pub(crate) fn build() -> Result<GuardrailToolParam, HeadroomError> {
    let tool = json!({
        "type": "function",
        "function": {
            "name": HEADROOM_RETRIEVE_TOOL_NAME,
            "description": TOOL_DESCRIPTION,
            "parameters": {
                "type": "object",
                "properties": {
                    "hash": {"type": "string", "description": HASH_DESCRIPTION},
                    "query": {"type": "string", "description": QUERY_DESCRIPTION},
                },
                "required": ["hash"],
            },
        },
    });
    Ok(serde_json::from_value(tool)?)
}

pub(crate) fn has_retrieve_tool(tools: &[GuardrailToolParam]) -> bool {
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
    let Some(tools) = existing else {
        return Ok(vec![build()?]);
    };
    if has_retrieve_tool(tools) {
        return Ok(tools.to_vec());
    }
    let mut merged = tools.to_vec();
    merged.push(build()?);
    Ok(merged)
}
