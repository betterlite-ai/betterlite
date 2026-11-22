use betterlite_models::identity::provider_id::ProviderId;

mod tests;

// The config class LiteLLM's get_provider_files_config builds for a provider; a caller matches on the tag and builds its own config
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileProviderConfig {
    GoogleAiStudio,
    VertexAi,
    Bedrock,
    Manus,
    Anthropic,
    Mistral,
    Xai,
}

impl FileProviderConfig {
    #[must_use]
    pub const fn class_name(self) -> &'static str {
        match self {
            Self::GoogleAiStudio => "GoogleAIStudioFilesHandler",
            Self::VertexAi => "VertexAIFilesConfig",
            Self::Bedrock => "BedrockFilesConfig",
            Self::Manus => "ManusFilesConfig",
            Self::Anthropic => "AnthropicFilesConfig",
            Self::Mistral => "MistralFilesConfig",
            Self::Xai => "XAIFilesConfig",
        }
    }
}

const FILE_CONFIG_TAGS: &[(ProviderId, FileProviderConfig)] = &[
    (ProviderId::Gemini, FileProviderConfig::GoogleAiStudio),
    (ProviderId::VertexAi, FileProviderConfig::VertexAi),
    (ProviderId::Bedrock, FileProviderConfig::Bedrock),
    (ProviderId::Manus, FileProviderConfig::Manus),
    (ProviderId::Anthropic, FileProviderConfig::Anthropic),
    (ProviderId::Mistral, FileProviderConfig::Mistral),
    (ProviderId::Xai, FileProviderConfig::Xai),
];

// LiteLLM's ProviderConfigManager.get_provider_files_config: the model is accepted and unused, and a provider it has no files config for answers None
#[must_use]
pub fn file_config_selection(_model: &str, provider: ProviderId) -> Option<FileProviderConfig> {
    FILE_CONFIG_TAGS
        .iter()
        .find_map(|(tagged, config)| (*tagged == provider).then_some(*config))
}

// LiteLLM's supports_httpx_timeout: an HTTP timeout object survives only for the exact three names, and an absent provider keeps none
#[must_use]
pub fn file_timeout_capability(provider: Option<&str>) -> bool {
    matches!(provider, Some("openai" | "azure" | "bedrock"))
}
