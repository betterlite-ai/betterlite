use betterlite_prompts::arguments::json::Json;
use serde::Serialize;

use super::beta_headers::tool_kind;

const CODE_EXECUTION: &str = "code_execution";
const CODE_EXECUTION_TYPE: &str = "code_execution_20250522";
const CONTAINER_UPLOAD: &str = "container_upload";
const CONTENT: &str = "content";

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
) -> Result<(), serde_json::Error> {
    if !has_container_upload(messages) {
        return Ok(());
    }
    if tools
        .iter()
        .any(|tool| tool_kind(tool).is_some_and(|kind| kind.starts_with(CODE_EXECUTION)))
    {
        return Ok(());
    }
    let tool = AnthropicCodeExecutionTool {
        name: CODE_EXECUTION.to_owned(),
        kind: CODE_EXECUTION_TYPE.to_owned(),
    };
    tools.push(serde_json::from_str(&serde_json::to_string(&tool)?)?);
    Ok(())
}

fn has_container_upload(messages: &[Json]) -> bool {
    messages.iter().any(|message| {
        match message.as_object().and_then(|fields| fields.get(CONTENT)) {
            Some(Json::List(blocks)) => blocks
                .iter()
                .any(|block| tool_kind(block) == Some(CONTAINER_UPLOAD)),
            Some(_) | None => false,
        }
    })
}
