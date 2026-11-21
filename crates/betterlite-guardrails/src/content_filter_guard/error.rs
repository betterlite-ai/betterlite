use crate::guardrail_base::error::GuardrailBaseError;
use crate::pattern_matching::PatternError;
use crate::pattern_matching::blocked_words::WordsError;

#[derive(Debug, thiserror::Error)]
pub enum ContentFilterBuildError {
    #[error(transparent)]
    Base(#[from] GuardrailBaseError),
    #[error(transparent)]
    Pattern(#[from] PatternError),
    #[error(transparent)]
    BlockedWords(#[from] WordsError),
}
