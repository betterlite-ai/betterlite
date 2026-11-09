use std::collections::BTreeMap;

use betterlite_mcp::sampling_convert::messages::SamplingMessage;
use betterlite_mcp::sampling_convert::model_preferences::{ModelInfo, ModelPreferences};
use betterlite_mcp::sampling_convert::tools::{McpTool, McpToolChoice};
use betterlite_openai::chat::response::ChatResponse;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    Resolve(ResolveInput),
    Messages(MessagesInput),
    Tools(ToolsInput),
    Response(ResponseInput),
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResolveInput {
    #[serde(default)]
    pub scenarios: Vec<ResolveScenario>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResolveScenario {
    #[serde(default)]
    pub available_models: Vec<String>,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default)]
    pub fallback_model: Option<String>,
    #[serde(default)]
    pub model_info: BTreeMap<String, ModelInfo>,
    #[serde(default)]
    pub preferences: Option<ModelPreferences>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MessagesInput {
    #[serde(default)]
    pub scenarios: Vec<MessageScenario>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MessageScenario {
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub messages: Vec<SamplingMessage>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ToolsInput {
    #[serde(default)]
    pub scenarios: Vec<ToolScenario>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ToolScenario {
    #[serde(default)]
    pub tools: Option<Vec<McpTool>>,
    #[serde(default)]
    pub tool_choice: Option<McpToolChoice>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponseInput {
    #[serde(default)]
    pub scenarios: Vec<ResponseScenario>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponseScenario {
    pub model_name: String,
    pub response: ChatResponse,
}
