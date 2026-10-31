use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct McpTool {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub input_schema: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OpenAiTool {
    pub r#type: String,
    pub function: OpenAiFunction,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OpenAiFunction {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    pub strict: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ResponsesTool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    pub strict: bool,
    pub r#type: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AnthropicTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub r#type: String,
}

pub fn mcp_input_schema_normalized(input_schema: &Value) -> Value {
    let mut normalized = match input_schema {
        Value::Object(schema) => schema.clone(),
        _ => Map::new(),
    };
    normalized
        .entry("type".to_owned())
        .or_insert_with(|| Value::String("object".to_owned()));
    normalized
        .entry("properties".to_owned())
        .or_insert_with(|| Value::Object(Map::new()));
    normalized
        .entry("additionalProperties".to_owned())
        .or_insert(Value::Bool(false));
    Value::Object(normalized)
}

pub fn mcp_tool_to_openai(tool: &McpTool) -> OpenAiTool {
    OpenAiTool {
        r#type: "function".to_owned(),
        function: OpenAiFunction {
            name: tool.name.clone(),
            description: tool.description.clone().unwrap_or_default(),
            parameters: mcp_input_schema_normalized(&tool.input_schema),
            strict: false,
        },
    }
}

pub fn mcp_tool_to_responses_api(tool: &McpTool) -> ResponsesTool {
    ResponsesTool {
        name: tool.name.clone(),
        description: tool.description.clone().unwrap_or_default(),
        parameters: mcp_input_schema_normalized(&tool.input_schema),
        strict: false,
        r#type: "function".to_owned(),
    }
}

pub fn mcp_tool_to_anthropic(tool: &McpTool) -> AnthropicTool {
    AnthropicTool {
        name: tool.name.clone(),
        description: tool.description.clone().unwrap_or_default(),
        input_schema: sanitize_anthropic_schema(&tool.input_schema),
        r#type: "custom".to_owned(),
    }
}

fn sanitize_anthropic_schema(schema: &Value) -> Value {
    match schema {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .filter(|(key, _)| key.as_str() != "additionalProperties")
                .map(|(key, value)| (key.clone(), sanitize_anthropic_schema(value)))
                .collect(),
        ),
        Value::Array(values) => {
            Value::Array(values.iter().map(sanitize_anthropic_schema).collect())
        }
        value => value.clone(),
    }
}
