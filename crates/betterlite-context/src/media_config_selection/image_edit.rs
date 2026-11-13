use betterlite_models::identity::provider_id::ProviderId;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum ImageEditConfigTag {
    OpenAi(String),
    HostedVllm(String),
    Azure,
    Recraft,
    BlackForestLabs,
    FalAi(String),
    AzureAi(String),
    Gemini(String),
    LiteLlmProxy,
    VertexAi(String),
    Stability(String),
    Bedrock(String),
    OpenRouter(String),
}

impl ImageEditConfigTag {
    #[must_use]
    pub const fn class_name(&self) -> &'static str {
        match self {
            Self::OpenAi(_) => "OpenAIImageEditConfig",
            Self::HostedVllm(_) => "HostedVLLMImageEditConfig",
            Self::Azure => "AzureImageEditConfig",
            Self::Recraft => "RecraftImageEditConfig",
            Self::BlackForestLabs => "BlackForestLabsImageEditConfig",
            Self::FalAi(_) => "FalAIImageEditConfig",
            Self::AzureAi(_) => "AzureAIImageEditConfig",
            Self::Gemini(_) => "GeminiImageEditConfig",
            Self::LiteLlmProxy => "LiteLLMProxyImageEditConfig",
            Self::VertexAi(_) => "VertexAIImageEditConfig",
            Self::Stability(_) => "StabilityImageEditConfig",
            Self::Bedrock(_) => "BedrockImageEditConfig",
            Self::OpenRouter(_) => "OpenRouterImageEditConfig",
        }
    }
}

type TagBuilder = fn(String) -> ImageEditConfigTag;

const MODEL_TAGS: &[(ProviderId, TagBuilder)] = &[
    (ProviderId::Openai, ImageEditConfigTag::OpenAi),
    (ProviderId::HostedVllm, ImageEditConfigTag::HostedVllm),
    (ProviderId::FalAi, ImageEditConfigTag::FalAi),
    (ProviderId::AzureAi, ImageEditConfigTag::AzureAi),
    (ProviderId::Gemini, ImageEditConfigTag::Gemini),
    (ProviderId::VertexAi, ImageEditConfigTag::VertexAi),
    (ProviderId::Stability, ImageEditConfigTag::Stability),
    (ProviderId::Bedrock, ImageEditConfigTag::Bedrock),
    (ProviderId::Openrouter, ImageEditConfigTag::OpenRouter),
];

const DIRECT_TAGS: &[(ProviderId, ImageEditConfigTag)] = &[
    (ProviderId::Azure, ImageEditConfigTag::Azure),
    (ProviderId::Recraft, ImageEditConfigTag::Recraft),
    (
        ProviderId::BlackForestLabs,
        ImageEditConfigTag::BlackForestLabs,
    ),
    (ProviderId::LitellmProxy, ImageEditConfigTag::LiteLlmProxy),
];

#[must_use]
pub fn select_image_edit(model: &str, provider: ProviderId) -> Option<ImageEditConfigTag> {
    if let Some((_, tag)) = DIRECT_TAGS.iter().find(|(row, _)| *row == provider) {
        return Some(tag.clone());
    }
    MODEL_TAGS
        .iter()
        .find_map(|(row, build)| (*row == provider).then(|| build(model.to_owned())))
}
