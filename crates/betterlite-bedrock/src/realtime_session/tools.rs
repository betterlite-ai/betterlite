use serde::Deserialize;
use serde_json::value::RawValue;

use super::records::{Setting, Tool, ToolInputSchema, ToolSpec, nullable_setting};

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ToolDefinition {
    #[serde(default, rename = "type")]
    pub tool_type: Option<String>,
    #[serde(default)]
    pub function: Option<FunctionDefinition>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct FunctionDefinition {
    #[serde(default, deserialize_with = "nullable_setting")]
    pub name: Option<Setting<String>>,
    #[serde(default, deserialize_with = "nullable_setting")]
    pub description: Option<Setting<String>>,
    #[serde(default, deserialize_with = "nullable_setting")]
    pub parameters: Option<Setting<Box<RawValue>>>,
}

#[must_use]
pub fn transform(tools: &[ToolDefinition]) -> Vec<Tool> {
    let mut transformed = Vec::new();
    for tool in tools {
        if tool.tool_type.as_deref() != Some("function") {
            continue;
        }
        transformed.push(transform_one(tool));
    }
    transformed
}

fn transform_one(tool: &ToolDefinition) -> Tool {
    let definition = tool.function.as_ref();
    let name = setting_text(definition.and_then(|function| function.name.as_ref()));
    let description = setting_text(definition.and_then(|function| function.description.as_ref()));
    let json = match definition.and_then(|function| function.parameters.as_ref()) {
        Some(Setting(Some(parameters))) => parameters.get().to_owned(),
        Some(Setting(None)) => "null".to_owned(),
        None => "{}".to_owned(),
    };
    Tool {
        tool_spec: ToolSpec {
            name,
            description,
            input_schema: ToolInputSchema { json },
        },
    }
}

fn setting_text(value: Option<&Setting<String>>) -> Option<String> {
    match value {
        Some(Setting(Some(text))) => Some(text.clone()),
        Some(Setting(None)) => None,
        None => Some(String::new()),
    }
}
