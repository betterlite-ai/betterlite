use std::str::Utf8Error;

use thiserror::Error;

use super::verdict::{Boundary, Rule};

#[derive(Debug, Error)]
pub enum ForecastError {
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
    #[error("the classifier verdict crux must contain non-whitespace text")]
    BlankCrux,
    #[error("primary_rule {rule:?} requires capability_boundary {expected:?}, got {found:?}")]
    RuleBoundary {
        rule: Rule,
        expected: Boundary,
        found: Boundary,
    },
    #[error("p_solve must be a finite number between 0 and 1, got {value}")]
    SolveProbability { value: f64 },
    #[error("base_threshold must be a finite number between 0 and 1, got {value}")]
    BaseThreshold { value: f64 },
    #[error("threshold_step must be a finite number at least 0, got {value}")]
    ThresholdStep { value: f64 },
    #[error("base_threshold + 2 * threshold_step must be at most 1")]
    ThresholdRange,
    #[error("max_output_tokens must be at least 1")]
    MaxOutputTokens,
    #[error("efficient_tier must be non-empty")]
    EmptyEfficientTier,
    #[error("capable_tier must be non-empty")]
    EmptyCapableTier,
    #[error("calibration.version must be 1 to 128 non-space characters, no outer whitespace")]
    CalibrationVersion,
    #[error("calibration.slope must be a finite number between 0 and 20, got {value}")]
    CalibrationSlope { value: f64 },
    #[error("calibration.intercept must be a finite number between -20 and 20, got {value}")]
    CalibrationIntercept { value: f64 },
    #[error("the classifier prompt schema could not be serialized")]
    Serialization {
        #[source]
        source: serde_json::Error,
    },
}
