use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ModalityField {
    Text,
    Audio,
    Image,
    Video,
}

impl ModalityField {
    const fn name(self) -> &'static str {
        match self {
            Self::Text => "text_tokens",
            Self::Audio => "audio_tokens",
            Self::Image => "image_tokens",
            Self::Video => "video_tokens",
        }
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum CostUsageError {
    #[error("an integer counter does not fit a signed 64-bit value")]
    CounterOverflow,
    #[error("a truthy Interactions modality list is not an array")]
    ModalityListNotArray,
    #[error("an Interactions modality usage entry is not a mapping")]
    ModalityEntryNotObject,
    #[error("the {field} modality token sum overflowed", field = field.name())]
    ModalitySumOverflow { field: ModalityField },
    #[error("subtracting cached tokens from {field} overflowed", field = field.name())]
    CacheSubtractionOverflow { field: ModalityField },
    #[error("the Google search request count overflowed")]
    SearchCountOverflow,
    #[error("the prompt token total overflowed")]
    PromptTotalOverflow,
    #[error("the completion token total overflowed")]
    CompletionTotalOverflow,
    #[error("the total token count overflowed")]
    TotalTokensOverflow,
}
