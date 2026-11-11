use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Number, Value};

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Contents {
    One(Content),
    Many(Vec<Content>),
}

#[derive(Debug, Clone, Deserialize)]
pub struct Content {
    pub role: Option<String>,
    #[serde(default)]
    pub parts: Option<Vec<Part>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Part {
    Text(String),
    Fields(Box<PartFields>),
    Other(Value),
}

#[derive(Debug, Clone, Deserialize)]
pub struct PartFields {
    pub text: Option<String>,
    pub inline_data: Option<InlineData>,
    #[serde(rename = "functionResponse")]
    pub function_response: Option<Call>,
    #[serde(rename = "functionCall")]
    pub function_call: Option<Call>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InlineData {
    pub mime_type: Option<String>,
    pub data: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Call {
    pub name: Option<String>,
    pub args: Option<Value>,
    pub response: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GenAiTool {
    #[serde(rename = "functionDeclarations", default)]
    pub function_declarations: Option<Vec<FunctionDeclaration>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionDeclaration {
    pub name: Option<String>,
    pub description: Option<String>,
    pub parameters: Option<Value>,
    pub parameters_json_schema: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolConfig {
    #[serde(rename = "functionCallingConfig")]
    pub function_calling_config: Option<FunctionCallingConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FunctionCallingConfig {
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SystemInstruction {
    #[serde(default)]
    pub parts: Option<Vec<SystemPart>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SystemPart {
    pub text: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationConfig {
    pub temperature: Option<Number>,
    pub max_output_tokens: Option<u32>,
    pub top_p: Option<Number>,
    pub stop_sequences: Option<Vec<String>>,
    pub response_mime_type: Option<String>,
    #[serde(rename = "response_mime_type")]
    pub response_mime_type_snake: Option<String>,
    pub response_json_schema: Option<Value>,
    #[serde(rename = "response_json_schema")]
    pub response_json_schema_snake: Option<Value>,
    pub response_schema: Option<Value>,
    #[serde(rename = "response_schema")]
    pub response_schema_snake: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerateLiteLLMParams {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_version: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub max_retries: Option<u32>,
    pub organization: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GenerateContentRequest {
    pub model: String,
    pub contents: Contents,
    #[serde(rename = "systemInstruction")]
    pub system_instruction: Option<SystemInstruction>,
    #[serde(rename = "system_instruction")]
    pub system_instruction_snake: Option<SystemInstruction>,
    #[serde(default)]
    pub tools: Option<Vec<GenAiTool>>,
    #[serde(rename = "toolConfig")]
    pub tool_config: Option<ToolConfig>,
    #[serde(rename = "tool_config")]
    pub tool_config_snake: Option<ToolConfig>,
    #[serde(rename = "generationConfig")]
    pub generation_config: Option<GenerationConfig>,
}
