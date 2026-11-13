use serde::Deserialize;
use serde_json::value::RawValue;

use super::records::{Setting, Tool, ToolInputSchema, ToolSpec};

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ToolDefinition {
    #[serde(default, rename = "type")]
    pub tool_type: Option<String>,
    #[serde(default)]
    pub function: Option<FunctionDefinition>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct FunctionDefinition {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
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
    let function = tool.function.as_ref();
    let name = match function {
        Some(function) => function.name.clone().unwrap_or_default(),
        None => String::new(),
    };
    let description = match function {
        Some(function) => function.description.clone().unwrap_or_default(),
        None => String::new(),
    };
    let json = match function.and_then(|function| function.parameters.as_ref()) {
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
