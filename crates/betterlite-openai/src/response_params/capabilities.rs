use std::future::Future;

use betterlite_models::identity::provider_id::ProviderId;
use serde::{Deserialize, Serialize};

use crate::reasoning_models::ReasoningCapabilities;

pub const CUSTOM_LLM_PROVIDER: ProviderId = ProviderId::Openai;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub custom_llm_provider: ProviderId,
    pub supports_native_file_search: bool,
    #[serde(rename = "supports_encrypted_agent_messages")]
    pub encrypted_agent_messages: bool,
}

#[must_use]
pub const fn provider_capabilities(provider: ProviderId) -> ProviderCapabilities {
    let encrypted_agent_messages = matches!(provider, ProviderId::Openai | ProviderId::Azure);
    ProviderCapabilities {
        custom_llm_provider: provider,
        supports_native_file_search: true,
        encrypted_agent_messages,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelInfo {
    // a lookup that raised still permits reasoning, as LiteLLM's blanket except does
    Unavailable,
    Found {
        key: String,
        supports_reasoning: Option<bool>,
    },
}

pub trait ResponsesCapabilities: ReasoningCapabilities {
    fn model_info(&mut self, model: &str) -> impl Future<Output = ModelInfo> + Send;
}
