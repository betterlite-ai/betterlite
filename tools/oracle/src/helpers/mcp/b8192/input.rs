use std::collections::BTreeMap;

use betterlite_mcp::sampling_convert::messages::SamplingMessage;
use betterlite_mcp::sampling_convert::model_preferences::{ModelInfo, ModelPreferences};
use betterlite_mcp::sampling_convert::tools::{McpTool, McpToolChoice};
use betterlite_openai::chat::response::ChatResponse;
use serde::Deserialize;
use serde::de::Error as _;
use serde_json::Value;

// an internally tagged enum buffers its payload where a RawValue cannot be read, so dispatch here
#[derive(Debug, Deserialize)]
#[serde(try_from = "Value")]
pub(crate) enum Input {
    Resolve(ResolveInput),
    Messages(MessagesInput),
    Tools(ToolsInput),
    Response(ResponseInput),
}

impl TryFrom<Value> for Input {
    type Error = serde_json::Error;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let operation = value
            .get("operation")
            .and_then(Value::as_str)
            .ok_or_else(|| serde_json::Error::custom("the helper input names no operation"))?;
        let input = match operation {
            "resolve" => Self::Resolve(serde_json::from_value(value)?),
            "messages" => Self::Messages(serde_json::from_value(value)?),
            "tools" => Self::Tools(serde_json::from_value(value)?),
            "response" => Self::Response(serde_json::from_value(value)?),
            other => {
                return Err(serde_json::Error::custom(format!(
                    "unknown b8192 operation: {other}"
                )));
            }
        };
        Ok(input)
    }
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
