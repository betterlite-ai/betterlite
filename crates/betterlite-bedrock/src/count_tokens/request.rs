use betterlite_models::decisions::Json;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CountTokensRequest {
    pub input: CountTokensInput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CountTokensInput {
    Converse(ConverseInput),
    InvokeModel(InvokeModelInput),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConverseInput {
    pub messages: Vec<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<Vec<Json>>,
    #[serde(rename = "toolConfig", skip_serializing_if = "Option::is_none")]
    pub tool_config: Option<ToolConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolConfig {
    pub tools: Vec<Json>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InvokeModelInput {
    pub body: String,
}
