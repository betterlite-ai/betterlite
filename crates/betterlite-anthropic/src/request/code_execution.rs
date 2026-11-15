use betterlite_prompts::arguments::json::Json;
use serde::Serialize;

use super::convert::{self, ConvertError};

const CODE_EXECUTION: &str = "code_execution";
const CODE_EXECUTION_TYPE: &str = "code_execution_20250522";
const CONTAINER_UPLOAD: &str = "container_upload";
const CONTENT: &str = "content";
const TYPE: &str = "type";

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AnthropicCodeExecutionTool {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
}

// A container_upload block needs the code_execution tool, which the caller may already have declared
pub fn add_code_execution_tool(
    messages: &[Json],
    tools: &mut Vec<Json>,
) -> Result<(), ConvertError> {
    if !has_container_upload(messages) {
        return Ok(());
    }
    if tools.iter().any(has_code_execution_type) {
        return Ok(());
    }
    let tool = AnthropicCodeExecutionTool {
        name: CODE_EXECUTION.to_owned(),
        kind: CODE_EXECUTION_TYPE.to_owned(),
    };
    tools.push(convert::encode::<_, Json>(&tool)?);
    Ok(())
}

fn has_container_upload(messages: &[Json]) -> bool {
    for message in messages {
        let Some(Json::List(blocks)) = message.as_object().and_then(|fields| fields.get(CONTENT))
        else {
            continue;
        };
        if blocks.iter().any(is_container_upload) {
            return true;
        }
    }
    false
}

fn is_container_upload(block: &Json) -> bool {
    match block
        .as_object()
        .and_then(|fields| fields.get(TYPE))
        .and_then(Json::as_str)
    {
        Some(kind) => kind == CONTAINER_UPLOAD,
        None => false,
    }
}

fn has_code_execution_type(tool: &Json) -> bool {
    match tool
        .as_object()
        .and_then(|fields| fields.get(TYPE))
        .and_then(Json::as_str)
    {
        Some(kind) => kind.starts_with(CODE_EXECUTION),
        None => false,
    }
}
