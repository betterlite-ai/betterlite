use crate::schema::SchemaLimits;
use crate::schema::normalize::normalize_json_schema_types;
use betterlite_openai::chat::{FunctionDefinition, Tool, ToolChoice};
use indexmap::IndexMap;

use super::GenAiInputError;
use super::input::{GenAiTool, ToolConfig};

pub(super) fn tool_choice(config: Option<&ToolConfig>) -> Option<ToolChoice> {
    let mode = config?
        .function_calling_config
        .as_ref()
        .and_then(|function_calling_config| function_calling_config.mode.as_deref());
    let choice = match mode {
        Some("ANY") => "required",
        Some("NONE") => "none",
        // LiteLLM's map holds AUTO and falls back to auto for a mode it does not know
        Some(_) | None => "auto",
    };
    Some(ToolChoice::Mode(String::from(choice)))
}

pub(super) fn tools_to_openai(
    tools: &[GenAiTool],
    limits: &SchemaLimits,
) -> Result<Vec<Tool>, GenAiInputError> {
    let mut openai_tools = Vec::new();
    for tool in tools {
        for declaration in tool.function_declarations.as_deref().unwrap_or_default() {
            let given = declaration
                .parameters_json_schema
                .as_ref()
                .or(declaration.parameters.as_ref());
            let parameters = match given.filter(|value| value.is_object()) {
                Some(value) => Some(normalize_json_schema_types(value, limits)?),
                None => None,
            };
            openai_tools.push(Tool {
                kind: String::from("function"),
                function: Some(FunctionDefinition {
                    name: declaration.name.clone().unwrap_or_default(),
                    description: declaration.description.clone(),
                    parameters,
                    strict: None,
                }),
                passthrough: IndexMap::new(),
            });
        }
    }
    Ok(openai_tools)
}
