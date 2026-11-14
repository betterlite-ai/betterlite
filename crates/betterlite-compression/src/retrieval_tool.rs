use betterlite_anthropic::tools::{ToolsError, map_tools};
use betterlite_prompts::arguments::json::Json;
use serde_json::{Map, Value};

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
    let mut key = Map::new();
    key.insert("type".to_owned(), Value::String("string".to_owned()));
    key.insert(
        "description".to_owned(),
        Value::String("The identifier of the content to retrieve".to_owned()),
    );
    key.insert(
        "enum".to_owned(),
        Value::Array(available_keys.iter().cloned().map(Value::String).collect()),
    );
    let mut properties = Map::new();
    properties.insert("key".to_owned(), Value::Object(key));
    let mut parameters = Map::new();
    parameters.insert("type".to_owned(), Value::String("object".to_owned()));
    parameters.insert("properties".to_owned(), Value::Object(properties));
    parameters.insert(
        "required".to_owned(),
        Value::Array(vec![Value::String("key".to_owned())]),
    );
    let mut function = Map::new();
    function.insert(
        "name".to_owned(),
        Value::String(RETRIEVAL_TOOL_NAME.to_owned()),
    );
    function.insert(
        "description".to_owned(),
        Value::String(format!(
            "Retrieve the full content of a file or message that was compressed to save tokens. Use this when you need the complete content to answer accurately. Available keys: {}",
            available_keys.join(", ")
        )),
    );
    function.insert("parameters".to_owned(), Value::Object(parameters));
    let mut tool = Map::new();
    tool.insert("type".to_owned(), Value::String("function".to_owned()));
    tool.insert("function".to_owned(), Value::Object(function));
    Value::Object(tool)
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
