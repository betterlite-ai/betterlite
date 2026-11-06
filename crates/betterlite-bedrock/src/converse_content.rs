use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};

pub mod citations;
pub mod json_mode;
pub mod recovery;
mod tests;
pub mod translation;

#[derive(Debug, thiserror::Error)]
pub enum ConverseContentError {
    #[error("the Bedrock tool content is not valid JSON: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("a Bedrock toolUse block is not an object")]
    ToolUseObject,
    #[error("a Bedrock toolUse block has no {field} value")]
    ToolUseField { field: &'static str },
}

// The union LiteLLM declares for get_tool_call_names: a Converse tool block or an OpenAI tool param.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ToolBlock {
    #[serde(rename = "toolSpec", default)]
    pub tool_spec: Option<ToolSpec>,
    #[serde(default)]
    pub function: Option<NamedFunction>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ToolSpec {
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct NamedFunction {
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ToolCall {
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ToolCallFunction,
    pub index: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ToolCallFunction {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}

// A Converse content block; the four members LiteLLM projects out of one block may all be present together.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ContentBlock {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(rename = "toolUse", default)]
    pub tool_use: Option<Json>,
    #[serde(rename = "reasoningContent", default)]
    pub reasoning_content: Option<Json>,
    #[serde(rename = "citationsContent", default)]
    pub citations_content: Option<Json>,
}

pub fn decode_converse_json_tool(bytes: &[u8]) -> Result<Json, ConverseContentError> {
    match serde_json::from_slice(bytes) {
        Ok(value) => Ok(value),
        Err(source) => Err(ConverseContentError::Json { source }),
    }
}

#[must_use]
pub fn get_tool_call_names(tools: Option<&[ToolBlock]>) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for tool in tools.unwrap_or_default() {
        let candidates = [
            tool.tool_spec
                .as_ref()
                .and_then(|spec| spec.name.as_deref()),
            tool.function
                .as_ref()
                .and_then(|function| function.name.as_deref()),
        ];
        for name in candidates.into_iter().flatten() {
            if !names.iter().any(|seen| seen == name) {
                names.push(name.to_owned());
            }
        }
    }
    names
}
