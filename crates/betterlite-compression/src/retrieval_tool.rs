use betterlite_anthropic::tools::{ToolsError, map_tools};
use betterlite_prompts::arguments::json::Json;
use serde_json::{Value, json};

use crate::message_normalization::CallType;

pub const RETRIEVAL_TOOL_NAME: &str = "litellm_content_retrieve";

#[derive(Debug, thiserror::Error)]
pub enum RetrievalToolError {
    #[error("the retrieval tool definition could not be read as provider JSON")]
    Input(#[source] serde_json::Error),
    #[error("the retrieval tool could not be mapped to the Anthropic tool schema")]
    Anthropic(#[from] ToolsError),
    #[error("the retrieval tool definition could not be written as JSON")]
    Output(#[source] serde_json::Error),
}

pub fn build_retrieval_tool(available_keys: &[String]) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": RETRIEVAL_TOOL_NAME,
            "description": format!(
                "Retrieve the full content of a file or message that was compressed to save tokens. Use this when you need the complete content to answer accurately. Available keys: {}",
                available_keys.join(", ")
            ),
            "parameters": {
                "type": "object",
                "properties": {
                    "key": {
                        "type": "string",
                        "description": "The identifier of the content to retrieve",
                        "enum": available_keys,
                    }
                },
                "required": ["key"],
            },
        },
    })
}

pub fn build_retrieval_tools(
    available_keys: &[String],
    call_type: CallType,
) -> Result<Vec<Value>, RetrievalToolError> {
    if available_keys.is_empty() {
        return Ok(Vec::new());
    }
    let openai_tools = vec![build_retrieval_tool(available_keys)];
    if call_type != CallType::AnthropicMessages {
        return Ok(openai_tools);
    }
    let provider_tools = openai_tools
        .into_iter()
        .map(|tool| serde_json::from_value::<Json>(tool).map_err(RetrievalToolError::Input))
        .collect::<Result<Vec<Json>, RetrievalToolError>>()?;
    let mapping = map_tools(&provider_tools)?;
    mapping
        .tools
        .into_iter()
        .map(|tool| serde_json::to_value(tool).map_err(RetrievalToolError::Output))
        .collect()
}

mod tests;
