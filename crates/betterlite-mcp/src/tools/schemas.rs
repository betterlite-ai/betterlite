use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::definitions::error::DefinitionsError;
use betterlite_prompts::definitions::sanitize::sanitize_for_anthropic;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct McpTool {
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "inputSchema", alias = "input_schema")]
    pub input_schema: Option<Object>,
    #[serde(flatten)]
    pub extra: Object,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Function,
    Custom,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OpenAiTool {
    pub r#type: ToolKind,
    pub function: OpenAiFunction,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OpenAiFunction {
    pub name: String,
    pub description: String,
    pub parameters: Object,
    pub strict: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ResponsesTool {
    pub name: String,
    pub description: String,
    pub parameters: Object,
    pub strict: bool,
    pub r#type: ToolKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AnthropicTool {
    pub name: String,
    pub description: String,
    pub input_schema: Object,
    pub r#type: ToolKind,
}

#[must_use]
pub fn mcp_input_schema_normalized(input_schema: Option<&Object>) -> Object {
    let mut normalized = input_schema.cloned().unwrap_or_default();
    normalized
        .entry("type".to_owned())
        .or_insert_with(|| Json::Text("object".to_owned()));
    normalized
        .entry("properties".to_owned())
        .or_insert_with(|| Json::Object(Object::new()));
    normalized
        .entry("additionalProperties".to_owned())
        .or_insert(Json::Bool(false));
    normalized
}

#[must_use]
pub fn mcp_tool_to_openai(tool: &McpTool) -> OpenAiTool {
    let parameters = mcp_input_schema_normalized(tool.input_schema.as_ref());
    OpenAiTool {
        r#type: ToolKind::Function,
        function: OpenAiFunction {
            name: tool.name.clone(),
            description: tool.description.clone().unwrap_or_default(),
            parameters,
            strict: false,
        },
    }
}

#[must_use]
pub fn mcp_tool_to_responses_api(tool: &McpTool) -> ResponsesTool {
    let parameters = mcp_input_schema_normalized(tool.input_schema.as_ref());
    ResponsesTool {
        name: tool.name.clone(),
        description: tool.description.clone().unwrap_or_default(),
        parameters,
        strict: false,
        r#type: ToolKind::Function,
    }
}

pub fn mcp_tool_to_anthropic(tool: &McpTool) -> Result<AnthropicTool, DefinitionsError> {
    let schema = sanitize_for_anthropic(tool.input_schema.clone().unwrap_or_default())?;
    let input_schema = [
        ("type", Some(schema.schema_type)),
        ("properties", Some(schema.properties)),
        ("additionalProperties", schema.additional_properties),
        ("required", schema.required),
        ("$defs", schema.defs),
        ("strict", schema.strict),
    ]
    .into_iter()
    .filter_map(|(key, value)| value.map(|item| (key.to_owned(), item)))
    .collect();
    Ok(AnthropicTool {
        name: tool.name.clone(),
        description: tool.description.clone().unwrap_or_default(),
        input_schema,
        r#type: ToolKind::Custom,
    })
}
