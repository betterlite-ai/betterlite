use betterlite_models::identity::provider_id::ProviderId;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum MessagesConfigTag {
    Anthropic,
    Bedrock(String),
    BedrockMantle,
    VertexAi,
    AzureAi,
    Minimax,
}

impl MessagesConfigTag {
    #[must_use]
    pub const fn class_name(&self) -> &'static str {
        match self {
            Self::Anthropic => "AnthropicMessagesConfig",
            Self::Bedrock(_) => "BedrockMessagesConfig",
            Self::BedrockMantle => "BedrockMantleAnthropicMessagesConfig",
            Self::VertexAi => "VertexAIPartnerModelsAnthropicMessagesConfig",
            Self::AzureAi => "AzureAnthropicMessagesConfig",
            Self::Minimax => "MinimaxMessagesConfig",
        }
    }
}

#[must_use]
pub fn select_messages_config(model: &str, provider: ProviderId) -> Option<MessagesConfigTag> {
    if provider == ProviderId::Anthropic {
        return Some(MessagesConfigTag::Anthropic);
    }
    if provider == ProviderId::Bedrock {
        return Some(MessagesConfigTag::Bedrock(model.to_owned()));
    }
    if provider == ProviderId::Minimax {
        return Some(MessagesConfigTag::Minimax);
    }
    if !model.to_lowercase().contains("claude") {
        return None;
    }
    if provider == ProviderId::BedrockMantle {
        Some(MessagesConfigTag::BedrockMantle)
    } else if provider == ProviderId::VertexAi {
        Some(MessagesConfigTag::VertexAi)
    } else if provider == ProviderId::AzureAi {
        Some(MessagesConfigTag::AzureAi)
    } else {
        None
    }
}
