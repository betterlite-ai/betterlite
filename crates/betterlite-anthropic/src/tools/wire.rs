use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;

pub const CUSTOM_TYPE: &str = "custom";
pub const ADVISOR_TYPE: &str = "advisor_20260301";
pub const URL_TYPE: &str = "url";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChoiceKind {
    Auto,
    Any,
    Tool,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolChoice {
    #[serde(rename = "type")]
    pub kind: ChoiceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disable_parallel_tool_use: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ToolDefinition {
    Function(FunctionTool),
    Computer(ComputerTool),
    Hosted(HostedTool),
    Advisor(AdvisorTool),
    Native(Object),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FunctionTool {
    pub name: String,
    pub input_schema: Json,
    #[serde(rename = "type")]
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eager_input_streaming: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defer_loading: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_callers: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_examples: Option<Vec<Json>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ComputerTool {
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
    pub display_width_px: Json,
    pub display_height_px: Json,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_number: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defer_loading: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_callers: Option<Vec<String>>,
}

// every other field of a hosted tool is the caller's and goes back out as sent
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HostedTool {
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
    #[serde(flatten)]
    pub fields: Object,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AdvisorTool {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub name: &'static str,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_uses: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caching: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defer_loading: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_callers: Option<Vec<String>>,
}

// a descriptor goes out on the wire, so it holds the token plainly and derives no Debug
#[derive(Clone, Serialize)]
#[serde(untagged)]
pub enum McpServerTool {
    Descriptor(Object),
    Mapped(MappedServer),
}

#[derive(Clone, Serialize)]
pub struct MappedServer {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub url: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_configuration: Option<ToolConfiguration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolConfiguration {
    pub allowed_tools: Vec<String>,
}
