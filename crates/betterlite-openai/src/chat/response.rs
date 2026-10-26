use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::chat::ToolCall;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ChatResponse {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<ChatChoice>,
    // LiteLLM answers with zero usage when the provider sends none
    #[serde(default)]
    pub usage: ChatUsage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ChatChoice {
    pub index: u32,
    pub message: AssistantMessage,
    pub finish_reason: Option<String>,
    // per-token log probabilities pass through as the provider sent them
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<Value>,
    // LiteLLM always writes this key, and the OpenAI provider leaves it empty
    #[serde(default, skip_deserializing)]
    pub provider_specific_fields: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct AssistantMessage {
    pub role: String,
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    // url citations and audio replies are provider data the client reads as sent
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Vec<Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<Value>,
    #[serde(default, skip_serializing)]
    pub refusal: Option<String>,
    #[serde(default, skip_deserializing)]
    pub provider_specific_fields: MessageProviderFields,
}

// LiteLLM moves the provider's refusal here and always writes the key
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct MessageProviderFields {
    pub refusal: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ChatUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens_details: Option<BTreeMap<String, Option<u64>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_tokens_details: Option<BTreeMap<String, Option<u64>>>,
}

impl ChatResponse {
    #[must_use]
    pub fn for_client(mut self, requested_model: Option<&str>) -> Self {
        if let Some(model) = requested_model.filter(|model| !model.is_empty()) {
            model.clone_into(&mut self.model);
        }
        for choice in &mut self.choices {
            choice.message.provider_specific_fields.refusal = choice.message.refusal.take();
        }
        self
    }
}
