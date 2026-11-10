#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PartnerPrefix {
    Meta,
    DeepseekAi,
    Mistral,
    Codestral,
    Jamba,
    Claude,
    Qwen,
    GptOss,
    MinimaxAi,
    MoonshotAi,
    ZaiOrg,
    GemmaMaas,
}

impl PartnerPrefix {
    pub const ALL: [Self; 12] = [
        Self::Meta,
        Self::DeepseekAi,
        Self::Mistral,
        Self::Codestral,
        Self::Jamba,
        Self::Claude,
        Self::Qwen,
        Self::GptOss,
        Self::MinimaxAi,
        Self::MoonshotAi,
        Self::ZaiOrg,
        Self::GemmaMaas,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Meta => "meta/",
            Self::DeepseekAi => "deepseek-ai",
            Self::Mistral => "mistral",
            Self::Codestral => "codestral",
            Self::Jamba => "jamba",
            Self::Claude => "claude",
            Self::Qwen => "qwen",
            Self::GptOss => "openai/gpt-oss-",
            Self::MinimaxAi => "minimaxai/",
            Self::MoonshotAi => "moonshotai/",
            Self::ZaiOrg => "zai-org/",
            Self::GemmaMaas => "google/gemma-",
        }
    }
}

// LiteLLM picks the OpenAI-shaped handler by substring, which is a different question from the prefix list above
const OPENAI_HANDLER_PATTERNS: [&str; 8] = [
    "llama",
    "deepseek-ai",
    "qwen",
    "openai/gpt-oss-",
    "minimaxai/",
    "moonshotai/",
    "zai-org/",
    "google/gemma-",
];

#[must_use]
pub fn is_vertex_partner_model(model: &str) -> bool {
    for prefix in PartnerPrefix::ALL {
        if model.starts_with(prefix.as_str()) {
            return true;
        }
    }
    false
}

#[must_use]
pub fn should_use_openai_handler(model: &str) -> bool {
    for pattern in OPENAI_HANDLER_PATTERNS {
        if model.contains(pattern) {
            return true;
        }
    }
    false
}
