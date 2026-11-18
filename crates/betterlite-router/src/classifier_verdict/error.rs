use std::str::Utf8Error;

use betterlite_logging::logging_privacy::CallbackMetadataError;
use thiserror::Error;

use super::rubric::ClassificationRubric;

#[derive(Debug, Error)]
pub enum ClassifierVerdictError {
    #[error("the classifier reply is {bytes} bytes, over the {limit}-byte parsing limit")]
    ReplyTooLarge { bytes: usize, limit: usize },
    #[error("the classifier reply is not valid UTF-8")]
    ReplyNotText {
        #[source]
        source: Utf8Error,
    },
    #[error("the classifier reply contains no complete JSON object")]
    NoVerdictObject,
    #[error("the classifier verdict does not match its schema: {source}")]
    VerdictSchema {
        #[source]
        source: serde_json::Error,
    },
    #[error("the classifier returned an unrecognized tier: {label}")]
    UnknownTier { label: String },
    #[error("the {rubric:?} rubric ships no calibration examples")]
    RubricExamples { rubric: ClassificationRubric },
    #[error("the calibration examples name the {tier} tier, which the labeled tiers omit")]
    MissingLabel { tier: &'static str },
    #[error("tier {name} has no description and names no built-in tier")]
    TierCriterion { name: String },
    #[error("{field} must be non-empty; omit the field instead")]
    OperatorSectionBlank { field: &'static str },
    #[error("{field} exceeds {cap} characters")]
    OperatorSectionTooLong { field: &'static str, cap: usize },
    #[error("request kwargs could not be read for privacy: {source}")]
    RequestKwargs {
        #[source]
        source: serde_json::Error,
    },
    #[error("callback metadata could not be read for privacy: {source}")]
    CallbackMetadata {
        #[from]
        source: CallbackMetadataError,
    },
}
