use crate::model_policy::capabilities::VertexCapabilityProvider;

pub const X_GOOGLE_API_KEY: &str = "x-goog-api-key";

// GoogleGenAIConfig.get_supported_generate_content_optional_params returns this list for every model
#[rustfmt::skip]
pub const SUPPORTED_OPTIONS: [&str; 30] = [
    "http_options", "system_instruction", "temperature", "top_p", "top_k",
    "candidate_count", "max_output_tokens", "stop_sequences", "response_logprobs",
    "logprobs", "presence_penalty", "frequency_penalty", "seed", "response_mime_type",
    "response_schema", "response_json_schema", "routing_config", "model_selection_config",
    "safety_settings", "tools", "tool_config", "labels", "cached_content",
    "response_modalities", "media_resolution", "speech_config", "audio_timestamp",
    "automatic_function_calling", "thinking_config", "image_config",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeProvider {
    Studio,
    Vertex,
}

impl NativeProvider {
    // LiteLLM's custom_llm_provider on GoogleGenAIConfig and VertexAIGoogleGenAIConfig
    #[must_use]
    pub const fn custom_llm_provider(self) -> &'static str {
        match self {
            Self::Studio => "gemini",
            Self::Vertex => "vertex_ai",
        }
    }

    #[must_use]
    pub const fn capability_provider(self) -> VertexCapabilityProvider {
        match self {
            Self::Studio => VertexCapabilityProvider::Gemini,
            Self::Vertex => VertexCapabilityProvider::VertexAi,
        }
    }
}
