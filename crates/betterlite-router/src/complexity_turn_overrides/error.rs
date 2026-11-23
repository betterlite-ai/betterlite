use thiserror::Error;

use crate::classifier_message_text::error::ClassifierTextError;
use crate::complexity_scoring::error::ScoringError;

#[derive(Debug, Error)]
pub enum TurnOverrideError {
    #[error("tier {tier} is not one of the active tiers")]
    UnknownTier { tier: String },
    #[error(transparent)]
    ClassifierText(#[from] ClassifierTextError),
    #[error(transparent)]
    KeywordRule(#[from] ScoringError),
}
