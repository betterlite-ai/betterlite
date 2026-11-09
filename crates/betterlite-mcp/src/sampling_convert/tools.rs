use serde::Deserialize;

use super::json_object::JsonObject;
use super::openai_wire::{OpenAiFunctionDefinition, OpenAiTool, OpenAiToolChoice, ToolKind};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct McpTool {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "inputSchema", alias = "input_schema")]
    pub input_schema: JsonObject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct McpToolChoice {
    #[serde(default)]
    pub mode: Option<ToolChoiceMode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolChoiceMode {
    Auto,
    Required,
    None,
}

#[must_use]
pub fn convert_tools(tools: Option<&[McpTool]>) -> Option<Vec<OpenAiTool>> {
    let present = tools?;
    if present.is_empty() {
        return None;
    }
    Some(
        present
            .iter()
            .map(|tool| OpenAiTool {
                kind: ToolKind::Function,
                function: OpenAiFunctionDefinition {
                    name: tool.name.clone(),
                    description: tool.description.clone().unwrap_or_default(),
                    parameters: tool.input_schema.clone(),
                },
            })
            .collect(),
    )
}

#[must_use]
pub fn convert_tool_choice(tool_choice: Option<&McpToolChoice>) -> Option<OpenAiToolChoice> {
    let choice = tool_choice?;
    let mode = choice.mode.unwrap_or(ToolChoiceMode::Auto);
    Some(match mode {
        ToolChoiceMode::Auto => OpenAiToolChoice::Auto,
        ToolChoiceMode::Required => OpenAiToolChoice::Required,
        ToolChoiceMode::None => OpenAiToolChoice::None,
    })
}
