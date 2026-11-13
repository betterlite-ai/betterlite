use std::collections::BTreeMap;

use super::presence::Field;
use super::presence::optional;
use betterlite_prompts::arguments::json::Object;
use serde::{Deserialize, Serialize};

use super::messages::ChatCompletionTool;
use super::messages::ChatMessage;
use super::presence::Given;
use super::response_format::ResponseFormatChoice;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Template {
    pub template: Vec<ChatMessage>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub defaults: Field<BTreeMap<String, String>>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub response_format: Field<ResponseFormatChoice>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub tools: Field<Vec<ChatCompletionTool>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LLMModelDetails {
    pub name: String,
    #[serde(default, skip_serializing_if = "Given::is_missing")]
    pub version: Given<String>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub params: Field<Object>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PromptTemplatingModuleConfig {
    pub prompt: Template,
    pub model: LLMModelDetails,
}
