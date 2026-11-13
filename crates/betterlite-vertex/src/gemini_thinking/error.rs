use thiserror::Error;

#[derive(Debug, Error)]
pub enum GeminiThinkingError {
    #[error(
        "Invalid `reasoning_effort`: {effort}. Must be one of: 'minimal', 'low', 'medium', 'high', 'none', 'disable'"
    )]
    UnsupportedEffort { effort: String },
    #[error(
        "Cannot specify both `{param}` (which maps to `thinking_budget`) and `thinking_level` in the same request. For Gemini 3 models, use `thinking_level` instead."
    )]
    ThinkingLevelConflict { param: String },
    #[error(
        "Unsupported audio format for Gemini TTS models: {format}. Gemini TTS models only support 'pcm16' format as they return audio data in L16 PCM format."
    )]
    UnsupportedAudioFormat { format: String },
}

impl GeminiThinkingError {
    // LiteLLM raises UnsupportedParamsError with 400 for the effort and conflict checks, and a bare ValueError for audio
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::UnsupportedEffort { .. } | Self::ThinkingLevelConflict { .. } => Some(400),
            Self::UnsupportedAudioFormat { .. } => None,
        }
    }
}
