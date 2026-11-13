use thiserror::Error;

#[derive(Debug, Error)]
pub enum GeminiThinkingError {
    #[error(
        "Invalid `reasoning_effort`: {effort}. Must be one of: 'minimal', 'low', 'medium', 'high', 'none', 'disable'"
    )]
    UnsupportedEffort { effort: String },
    #[error(
        "Cannot specify both `{param}` and `thinking_level` in the same request. For Gemini 3 models, use `thinking_level` instead."
    )]
    ThinkingLevelConflict { param: String },
    #[error(
        "Unsupported audio format for Gemini TTS models: {format}. Gemini TTS models only support 'pcm16' format."
    )]
    UnsupportedAudioFormat { format: String },
}

impl GeminiThinkingError {
    // LiteLLM raises UnsupportedParamsError (400) for the effort and conflict checks, and a bare ValueError for audio that the exception mapper turns into a 500
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::UnsupportedEffort { .. } | Self::ThinkingLevelConflict { .. } => Some(400),
            Self::UnsupportedAudioFormat { .. } => Some(500),
        }
    }
}
