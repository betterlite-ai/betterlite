use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_with::skip_serializing_none;

use crate::chat::ToolCall;

#[skip_serializing_none]
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
    pub system_fingerprint: Option<String>,
    pub service_tier: Option<String>,
}

// logprobs pass through as the provider sent them, and provider_specific_fields stays empty for OpenAI
#[skip_serializing_none]
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ChatChoice {
    pub index: u32,
    pub message: AssistantMessage,
    #[serialize_always]
    pub finish_reason: Option<String>,
    pub logprobs: Option<Value>,
    #[serde(default, skip_deserializing)]
    pub provider_specific_fields: BTreeMap<String, Value>,
}

// annotations and audio are provider data the client reads as sent
#[skip_serializing_none]
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct AssistantMessage {
    pub role: String,
    #[serialize_always]
    pub content: Option<String>,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub annotations: Option<Vec<Value>>,
    pub audio: Option<Value>,
    #[serde(skip_serializing)]
    pub refusal: Option<String>,
    #[serde(default, skip_deserializing)]
    pub provider_specific_fields: MessageProviderFields,
}

// LiteLLM moves the provider's refusal here and always writes the key
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct MessageProviderFields {
    pub refusal: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ChatUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub prompt_tokens_details: Option<BTreeMap<String, Option<u64>>>,
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
