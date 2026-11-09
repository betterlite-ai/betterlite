use std::collections::BTreeMap;

use betterlite_error::facts::presence::Field;
use betterlite_prompts::arguments::json::Json;
use serde::{Deserialize, Serialize};

use super::messages::ChatCompletionTool;
use super::messages::ChatMessage;
use super::presence::Given;
use super::response_format::ResponseFormatChoice;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Template {
    pub template: Vec<ChatMessage>,
    #[serde(default, skip_serializing_if = "Field::is_missing")]
    pub defaults: Field<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Field::is_missing")]
    pub response_format: Field<ResponseFormatChoice>,
    #[serde(default, skip_serializing_if = "Field::is_missing")]
    pub tools: Field<Vec<ChatCompletionTool>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LLMModelDetails {
    pub name: String,
    #[serde(default, skip_serializing_if = "Given::is_missing")]
    pub version: Given<String>,
    #[serde(default, skip_serializing_if = "Field::is_missing")]
    pub params: Field<Json>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PromptTemplatingModuleConfig {
    pub prompt: Template,
    pub model: LLMModelDetails,
}
