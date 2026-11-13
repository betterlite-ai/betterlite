use betterlite_models::identity::provider_id::ProviderId;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum ImageGenerationConfigTag {
    OpenAi(String),
    Azure(String),
    AzureAi(String),
    Xinference(String),
    Recraft(String),
    Aiml(String),
    CometApi(String),
    Gemini(String),
    LiteLlmProxy,
    FalAi(String),
    Stability(String),
    RunwayMl(String),
    BlackForestLabs(String),
    VertexAi(String),
    OpenRouter(String),
    DashscopeFamily(String),
    ModelScope(String),
    EdenAi,
}

impl ImageGenerationConfigTag {
    #[must_use]
    pub const fn class_name(&self) -> &'static str {
        match self {
            Self::OpenAi(_) => "OpenAIImageGenerationConfig",
            Self::Azure(_) => "AzureImageGenerationConfig",
            Self::AzureAi(_) => "AzureAIImageGenerationConfig",
            Self::Xinference(_) => "XinferenceImageGenerationConfig",
            Self::Recraft(_) => "RecraftImageGenerationConfig",
            Self::Aiml(_) => "AimlImageGenerationConfig",
            Self::CometApi(_) => "CometAPIImageGenerationConfig",
            Self::Gemini(_) => "GeminiImageGenerationConfig",
            Self::LiteLlmProxy => "LiteLLMProxyImageGenerationConfig",
            Self::FalAi(_) => "FalAIImageGenerationConfig",
            Self::Stability(_) => "StabilityImageGenerationConfig",
            Self::RunwayMl(_) => "RunwayMLImageGenerationConfig",
            Self::BlackForestLabs(_) => "BlackForestLabsImageGenerationConfig",
            Self::VertexAi(_) => "VertexAIImageGenerationConfig",
            Self::OpenRouter(_) => "OpenRouterImageGenerationConfig",
            Self::DashscopeFamily(_) => "DashscopeFamilyImageGenerationConfig",
            Self::ModelScope(_) => "ModelScopeImageGenerationConfig",
            Self::EdenAi => "EdenAIImageGenerationConfig",
        }
    }
}

type TagBuilder = fn(String) -> ImageGenerationConfigTag;

const MODEL_TAGS: &[(ProviderId, TagBuilder)] = &[
    (ProviderId::Openai, ImageGenerationConfigTag::OpenAi),
    (ProviderId::Azure, ImageGenerationConfigTag::Azure),
    (ProviderId::AzureAi, ImageGenerationConfigTag::AzureAi),
    (ProviderId::Xinference, ImageGenerationConfigTag::Xinference),
    (ProviderId::Recraft, ImageGenerationConfigTag::Recraft),
    (ProviderId::Aiml, ImageGenerationConfigTag::Aiml),
    (ProviderId::Cometapi, ImageGenerationConfigTag::CometApi),
    (ProviderId::Gemini, ImageGenerationConfigTag::Gemini),
    (ProviderId::FalAi, ImageGenerationConfigTag::FalAi),
    (ProviderId::Stability, ImageGenerationConfigTag::Stability),
    (ProviderId::Runwayml, ImageGenerationConfigTag::RunwayMl),
    (
        ProviderId::BlackForestLabs,
        ImageGenerationConfigTag::BlackForestLabs,
    ),
    (ProviderId::VertexAi, ImageGenerationConfigTag::VertexAi),
    (ProviderId::Openrouter, ImageGenerationConfigTag::OpenRouter),
    (ProviderId::Modelscope, ImageGenerationConfigTag::ModelScope),
];

#[must_use]
pub fn select_image_generation(
    model: &str,
    provider: ProviderId,
) -> Option<ImageGenerationConfigTag> {
    if provider == ProviderId::Dashscope
        || provider == ProviderId::Qwencloud
        || provider == ProviderId::QwenAiPlatform
    {
        return Some(ImageGenerationConfigTag::DashscopeFamily(
            provider.to_string(),
        ));
    }
    if provider == ProviderId::LitellmProxy {
        return Some(ImageGenerationConfigTag::LiteLlmProxy);
    }
    if provider == ProviderId::Edenai {
        return Some(ImageGenerationConfigTag::EdenAi);
    }
    MODEL_TAGS
        .iter()
        .find_map(|(row, build)| (*row == provider).then(|| build(model.to_owned())))
}
