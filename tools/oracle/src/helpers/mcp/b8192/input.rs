use std::collections::BTreeMap;

use betterlite_mcp::sampling_convert::messages::SamplingMessage;
use betterlite_mcp::sampling_convert::model_preferences::{ModelInfo, ModelPreferences};
use betterlite_mcp::sampling_convert::tools::{McpTool, McpToolChoice};
use betterlite_openai::chat::response::ChatResponse;
use rust_decimal::Decimal;
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
    pub model_info: BTreeMap<String, ModelInfoInput>,
    #[serde(default)]
    pub preferences: Option<ModelPreferences>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ModelInfoInput {
    #[serde(default, with = "rust_decimal::serde::float_option")]
    pub input_cost_per_token: Option<Decimal>,
    #[serde(default, with = "rust_decimal::serde::float_option")]
    pub output_cost_per_token: Option<Decimal>,
    #[serde(default)]
    pub max_output_tokens: Option<u64>,
    #[serde(default)]
    pub max_tokens: Option<u64>,
    #[serde(default, with = "rust_decimal::serde::float_option")]
    pub output_tokens_per_second: Option<Decimal>,
}

impl ModelInfoInput {
    pub(crate) fn model_info(&self) -> ModelInfo {
        ModelInfo {
            input_cost_per_token: self.input_cost_per_token.unwrap_or_default(),
            output_cost_per_token: self.output_cost_per_token.unwrap_or_default(),
            max_output_tokens: self
                .max_output_tokens
                .filter(|value| *value != 0)
                .or(self.max_tokens)
                .unwrap_or(0),
            output_tokens_per_second: self.output_tokens_per_second.unwrap_or_default(),
        }
    }
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
