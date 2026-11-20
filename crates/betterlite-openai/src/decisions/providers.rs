#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecisionsProvider {
    Openai,
    Openrouter,
    AzureAi,
    Perplexity,
    Typesafe,
    StrandsDecider,
    Cloudflare,
    HostedVllm,
}

impl DecisionsProvider {
    // LlmProviders declares this order; get_provider_decisions_config checks perplexity first
    pub const ALL: [Self; 8] = [
        Self::Openai,
        Self::Openrouter,
        Self::AzureAi,
        Self::Perplexity,
        Self::Typesafe,
        Self::StrandsDecider,
        Self::Cloudflare,
        Self::HostedVllm,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Openai => "openai",
            Self::Openrouter => "openrouter",
            Self::AzureAi => "azure_ai",
            Self::Perplexity => "perplexity",
            Self::Typesafe => "typesafe",
            Self::StrandsDecider => "strands_decider",
            Self::Cloudflare => "cloudflare",
            Self::HostedVllm => "hosted_vllm",
        }
    }

    #[must_use]
    pub fn from_provider_name(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|provider| provider.as_str() == name)
    }
}

#[must_use]
pub fn supported_decisions_providers() -> Vec<&'static str> {
    DecisionsProvider::ALL
        .iter()
        .copied()
        .map(DecisionsProvider::as_str)
        .collect()
}

mod tests;
