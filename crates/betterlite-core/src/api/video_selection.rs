pub mod error;

use betterlite_models::identity::provider_id::ProviderId;

use self::error::VideoSelectionError;

// The config class LiteLLM's get_provider_video_config builds for a provider; a caller matches on the tag and builds its own adapter
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VideoConfigTag {
    OpenAi,
    Azure,
    Gemini,
    VertexAi,
    RunwayMl,
    FalAi,
    HostedVllm(Option<String>),
}

impl VideoConfigTag {
    #[must_use]
    pub const fn class_name(&self) -> &'static str {
        match self {
            Self::OpenAi => "OpenAIVideoConfig",
            Self::Azure => "AzureVideoConfig",
            Self::Gemini => "GeminiVideoConfig",
            Self::VertexAi => "VertexAIVideoConfig",
            Self::RunwayMl => "RunwayMLVideoConfig",
            Self::FalAi => "FalAIVideoConfig",
            Self::HostedVllm(_) => "HostedVLLMVideoConfig",
        }
    }
}

// The arms LiteLLM builds with no argument; hosted vLLM and Eden AI take their own check below
const PROVIDER_TAGS: &[(ProviderId, VideoConfigTag)] = &[
    (ProviderId::Openai, VideoConfigTag::OpenAi),
    (ProviderId::Azure, VideoConfigTag::Azure),
    (ProviderId::Gemini, VideoConfigTag::Gemini),
    (ProviderId::VertexAi, VideoConfigTag::VertexAi),
    (ProviderId::Runwayml, VideoConfigTag::RunwayMl),
    (ProviderId::FalAi, VideoConfigTag::FalAi),
];

// LiteLLM's ProviderConfigManager.get_provider_video_config: a provider with no video config at this pin is None, a known one whose config is not built yet is an error
#[must_use]
pub fn get_provider_video_config(
    model: Option<&str>,
    provider: ProviderId,
) -> Result<Option<VideoConfigTag>, VideoSelectionError> {
    // LiteLLM forwards the model only to get_hosted_vllm_video_config, which keeps it and builds the same config for every model
    if provider == ProviderId::HostedVllm {
        return Ok(Some(VideoConfigTag::HostedVllm(model.map(str::to_owned))));
    }
    // B5838 installs the Eden AI config; until then a provider that has one in LiteLLM never falls through to None
    if provider == ProviderId::Edenai {
        return Err(VideoSelectionError::Unavailable {
            provider: "edenai".to_owned(),
        });
    }
    Ok(PROVIDER_TAGS
        .iter()
        .find_map(|(row, tag)| (*row == provider).then(|| tag.clone())))
}

#[path = "video_selection/tests/azure.rs"]
mod azure;
#[path = "video_selection/tests/branch_coverage.rs"]
mod branch_coverage;
#[path = "video_selection/tests/edenai.rs"]
mod edenai;
#[path = "video_selection/tests/fal.rs"]
mod fal;
#[path = "video_selection/tests/gemini.rs"]
mod gemini;
#[path = "video_selection/tests/hosted_vllm.rs"]
mod hosted_vllm;
#[path = "video_selection/tests/openai.rs"]
mod openai;
#[path = "video_selection/tests/runway.rs"]
mod runway;
#[path = "video_selection/tests/signature.rs"]
mod signature;
#[path = "video_selection/tests/vertex.rs"]
mod vertex;
