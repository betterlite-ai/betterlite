use super::legacy::LEGACY_ANTHROPIC_MODEL_NAMES;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BedrockRoute {
    Invoke,
    ClaudePlatform,
    ConverseLike,
    Converse,
    Agent,
    Agentcore,
    AsyncInvoke,
    Openai,
    Mantle,
    ChatCompletions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BedrockInvokeProvider {
    Cohere,
    Anthropic,
    Mistral,
    Amazon,
    Meta,
    Llama,
    Ai21,
    Nova,
    DeepseekR1,
    Qwen3,
    Qwen2,
    Twelvelabs,
    Openai,
    Stability,
    Moonshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BedrockChatConfigTag {
    ClaudePlatform,
    Converse,
    OpenAi,
    RuntimeChatCompletions,
    InvokeAgent,
    AgentCore,
    Mantle,
    Titan,
    Anthropic,
    AnthropicClaude,
    Llama,
    Ai21,
    Cohere,
    Mistral,
    TwelveLabs,
    Moonshot,
    DeepSeekR1,
    Qwen3,
    Qwen2,
    Nova,
    Invoke,
}

impl BedrockChatConfigTag {
    #[must_use]
    pub const fn class_name(self) -> &'static str {
        match self {
            Self::ClaudePlatform => "BedrockClaudePlatformConfig",
            Self::Converse => "AmazonConverseConfig",
            Self::OpenAi => "AmazonBedrockOpenAIConfig",
            Self::RuntimeChatCompletions => "AmazonBedrockRuntimeChatCompletionsConfig",
            Self::InvokeAgent => "AmazonInvokeAgentConfig",
            Self::AgentCore => "AmazonAgentCoreConfig",
            Self::Mantle => "AmazonMantleConfig",
            Self::Titan => "AmazonTitanConfig",
            Self::Anthropic => "AmazonAnthropicConfig",
            Self::AnthropicClaude => "AmazonAnthropicClaudeConfig",
            Self::Llama => "AmazonLlamaConfig",
            Self::Ai21 => "AmazonAI21Config",
            Self::Cohere => "AmazonCohereConfig",
            Self::Mistral => "AmazonMistralConfig",
            Self::TwelveLabs => "AmazonTwelveLabsPegasusConfig",
            Self::Moonshot => "AmazonMoonshotConfig",
            Self::DeepSeekR1 => "AmazonDeepSeekR1Config",
            Self::Qwen3 => "AmazonQwen3Config",
            Self::Qwen2 => "AmazonQwen2Config",
            Self::Nova => "AmazonInvokeNovaConfig",
            Self::Invoke => "AmazonInvokeConfig",
        }
    }
}

// B598, B591 and B2203 compute the route, provider and base model; selection only maps them to the owned classes
#[must_use]
pub fn select_bedrock_chat_config(
    route: BedrockRoute,
    invoke_provider: Option<BedrockInvokeProvider>,
    base_model: &str,
) -> BedrockChatConfigTag {
    let explicit = match route {
        BedrockRoute::ClaudePlatform => Some(BedrockChatConfigTag::ClaudePlatform),
        BedrockRoute::Converse | BedrockRoute::ConverseLike => Some(BedrockChatConfigTag::Converse),
        BedrockRoute::Openai => Some(BedrockChatConfigTag::OpenAi),
        BedrockRoute::ChatCompletions => Some(BedrockChatConfigTag::RuntimeChatCompletions),
        BedrockRoute::Agent => Some(BedrockChatConfigTag::InvokeAgent),
        BedrockRoute::Agentcore => Some(BedrockChatConfigTag::AgentCore),
        BedrockRoute::Mantle => Some(BedrockChatConfigTag::Mantle),
        BedrockRoute::Invoke | BedrockRoute::AsyncInvoke => None,
    };
    if let Some(tag) = explicit {
        return tag;
    }
    match invoke_provider {
        Some(BedrockInvokeProvider::Amazon) => BedrockChatConfigTag::Titan,
        Some(BedrockInvokeProvider::Anthropic) => {
            if LEGACY_ANTHROPIC_MODEL_NAMES.contains(&base_model) {
                BedrockChatConfigTag::Anthropic
            } else {
                BedrockChatConfigTag::AnthropicClaude
            }
        }
        Some(BedrockInvokeProvider::Meta | BedrockInvokeProvider::Llama) => {
            BedrockChatConfigTag::Llama
        }
        Some(BedrockInvokeProvider::Ai21) => BedrockChatConfigTag::Ai21,
        Some(BedrockInvokeProvider::Cohere) => BedrockChatConfigTag::Cohere,
        Some(BedrockInvokeProvider::Mistral) => BedrockChatConfigTag::Mistral,
        Some(BedrockInvokeProvider::Moonshot) => BedrockChatConfigTag::Moonshot,
        Some(BedrockInvokeProvider::DeepseekR1) => BedrockChatConfigTag::DeepSeekR1,
        Some(BedrockInvokeProvider::Nova) => BedrockChatConfigTag::Nova,
        Some(BedrockInvokeProvider::Qwen3) => BedrockChatConfigTag::Qwen3,
        Some(BedrockInvokeProvider::Qwen2) => BedrockChatConfigTag::Qwen2,
        Some(BedrockInvokeProvider::Twelvelabs) => BedrockChatConfigTag::TwelveLabs,
        Some(BedrockInvokeProvider::Openai | BedrockInvokeProvider::Stability) | None => {
            BedrockChatConfigTag::Invoke
        }
    }
}
