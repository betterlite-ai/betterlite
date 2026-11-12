use crate::model_capability_predicates::{CapabilityPredicates, CapabilityRequest};

mod tests;

// The capability keys whose named LiteLLM wrappers each make one shared-factory call.
#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::VariantArray)]
pub enum ModelCapabilityFlag {
    WebSearch,
    UrlContext,
    AudioInput,
    PdfInput,
    AudioOutput,
    PromptCacheBreakpoint,
    ThinkingCachePreservation,
    ComputerUse,
    Vision,
    AnthropicThinkingPayload,
    EmbeddingImageInput,
}

impl ModelCapabilityFlag {
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::WebSearch => "supports_web_search",
            Self::UrlContext => "supports_url_context",
            Self::AudioInput => "supports_audio_input",
            Self::PdfInput => "supports_pdf_input",
            // LiteLLM's wrapper at the pin asks for supports_audio_input; the flag keeps its own key
            Self::AudioOutput => "supports_audio_output",
            Self::PromptCacheBreakpoint => "supports_prompt_cache_breakpoint",
            Self::ThinkingCachePreservation => "supports_thinking_cache_preservation",
            Self::ComputerUse => "supports_computer_use",
            Self::Vision => "supports_vision",
            Self::AnthropicThinkingPayload => "supports_anthropic_thinking_payload",
            Self::EmbeddingImageInput => "supports_embedding_image_input",
        }
    }
}

pub async fn supports<Predicates>(
    predicates: &mut Predicates,
    model: &str,
    custom_llm_provider: Option<&str>,
    flag: ModelCapabilityFlag,
) -> bool
where
    Predicates: CapabilityPredicates + Send,
{
    let request = CapabilityRequest {
        model,
        custom_llm_provider,
    };
    predicates.supports_factory(&request, flag.key()).await
}

pub async fn is_vision_explicitly_disabled<Predicates>(
    predicates: &mut Predicates,
    model: &str,
    custom_llm_provider: Option<&str>,
) -> bool
where
    Predicates: CapabilityPredicates + Send,
{
    let request = CapabilityRequest {
        model,
        custom_llm_provider,
    };
    predicates
        .is_explicitly_disabled_factory(&request, ModelCapabilityFlag::Vision.key())
        .await
}
