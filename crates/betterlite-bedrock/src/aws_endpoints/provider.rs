use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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

impl BedrockInvokeProvider {
    pub const ALL: [Self; 15] = [
        Self::Cohere,
        Self::Anthropic,
        Self::Mistral,
        Self::Amazon,
        Self::Meta,
        Self::Llama,
        Self::Ai21,
        Self::Nova,
        Self::DeepseekR1,
        Self::Qwen3,
        Self::Qwen2,
        Self::Twelvelabs,
        Self::Openai,
        Self::Stability,
        Self::Moonshot,
    ];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Cohere => "cohere",
            Self::Anthropic => "anthropic",
            Self::Mistral => "mistral",
            Self::Amazon => "amazon",
            Self::Meta => "meta",
            Self::Llama => "llama",
            Self::Ai21 => "ai21",
            Self::Nova => "nova",
            Self::DeepseekR1 => "deepseek_r1",
            Self::Qwen3 => "qwen3",
            Self::Qwen2 => "qwen2",
            Self::Twelvelabs => "twelvelabs",
            Self::Openai => "openai",
            Self::Stability => "stability",
            Self::Moonshot => "moonshot",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BedrockEmbeddingProvider {
    Cohere,
    Amazon,
    Twelvelabs,
    Nova,
}

impl BedrockEmbeddingProvider {
    pub const ALL: [Self; 4] = [Self::Cohere, Self::Amazon, Self::Twelvelabs, Self::Nova];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Cohere => "cohere",
            Self::Amazon => "amazon",
            Self::Twelvelabs => "twelvelabs",
            Self::Nova => "nova",
        }
    }
}

#[must_use]
pub fn invoke_provider(model: &str) -> Option<BedrockInvokeProvider> {
    let stripped = model.strip_prefix("invoke/").unwrap_or(model);
    if stripped.to_lowercase().contains("nova") {
        return Some(BedrockInvokeProvider::Nova);
    }
    if let Some(provider) = [stripped.split('.').next(), stripped.split('/').next()]
        .into_iter()
        .flatten()
        .find_map(|name| {
            BedrockInvokeProvider::ALL
                .into_iter()
                .find(|provider| provider.name() == name)
        })
    {
        return Some(provider);
    }
    BedrockInvokeProvider::ALL
        .into_iter()
        .find(|provider| stripped.contains(provider.name()))
}

#[must_use]
pub fn embedding_provider(model: &str) -> Option<BedrockEmbeddingProvider> {
    if model.to_lowercase().contains("nova") {
        return Some(BedrockEmbeddingProvider::Nova);
    }
    if model.contains('.') {
        let mut parts = model.split('.');
        let first = parts.next().unwrap_or_default();
        let second = parts.next();
        if let Some(provider) = [second, Some(first)]
            .into_iter()
            .flatten()
            .find_map(|name| {
                BedrockEmbeddingProvider::ALL
                    .into_iter()
                    .find(|provider| provider.name() == name)
            })
        {
            return Some(provider);
        }
    }
    BedrockEmbeddingProvider::ALL
        .into_iter()
        .find(|provider| model.contains(provider.name()))
}
