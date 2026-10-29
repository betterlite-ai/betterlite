use thiserror::Error;

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum CostUsageError {
    #[error("an Interactions modality usage entry is not a mapping")]
    ModalityEntryNotObject,
    #[error("the {field} modality token sum overflowed")]
    ModalitySumOverflow { field: &'static str },
    #[error("subtracting cached tokens from {field} overflowed")]
    CacheSubtractionOverflow { field: &'static str },
    #[error("the Google search request count overflowed")]
    SearchCountOverflow,
    #[error("the prompt token total overflowed")]
    PromptTotalOverflow,
    #[error("the completion token total overflowed")]
    CompletionTotalOverflow,
    #[error("the total token count overflowed")]
    TotalTokensOverflow,
}
