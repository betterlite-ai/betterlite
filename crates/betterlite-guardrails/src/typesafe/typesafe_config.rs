use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::generic_guardrail_contracts::UnreachableFallback;
use crate::typesafe::{
    DEFAULT_API_BASE, DEFAULT_MAX_RESULT_CHARS, DEFAULT_MIN_CHARS_TO_EVALUATE, DEFAULT_MODEL,
    DEFAULT_RELEVANCE_THRESHOLD,
};

#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum TypeSafeConfigError {
    #[error("relevance_threshold must be between 0.0 and 1.0, got {value}")]
    RelevanceThreshold { value: f64 },
    #[error("min_chars_to_evaluate must be at least 0, got {value}")]
    MinCharsToEvaluate { value: i64 },
    #[error("max_result_chars_in_state must be at least 1")]
    MaxResultChars,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "f64")]
pub struct RelevanceThreshold(pub f64);

impl TryFrom<f64> for RelevanceThreshold {
    type Error = TypeSafeConfigError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        if (0.0_f64..=1.0_f64).contains(&value) {
            Ok(Self(value))
        } else {
            Err(TypeSafeConfigError::RelevanceThreshold { value })
        }
    }
}

impl Default for RelevanceThreshold {
    fn default() -> Self {
        Self(DEFAULT_RELEVANCE_THRESHOLD)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "i64")]
pub struct MinCharsToEvaluate(pub usize);

impl TryFrom<i64> for MinCharsToEvaluate {
    type Error = TypeSafeConfigError;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        usize::try_from(value)
            .map(Self)
            .map_err(|_overflow| TypeSafeConfigError::MinCharsToEvaluate { value })
    }
}

impl Default for MinCharsToEvaluate {
    fn default() -> Self {
        Self(DEFAULT_MIN_CHARS_TO_EVALUATE)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "usize")]
pub struct MaxResultChars(pub usize);

impl TryFrom<usize> for MaxResultChars {
    type Error = TypeSafeConfigError;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        if value == 0 {
            Err(TypeSafeConfigError::MaxResultChars)
        } else {
            Ok(Self(value))
        }
    }
}

impl Default for MaxResultChars {
    fn default() -> Self {
        Self(DEFAULT_MAX_RESULT_CHARS)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TypeSafeOptionalParams {
    pub relevance_threshold: Option<RelevanceThreshold>,
    pub min_chars_to_evaluate: Option<MinCharsToEvaluate>,
    pub max_result_chars_in_state: Option<MaxResultChars>,
}

// The api_key field stays plain text here because this is the config vocabulary the UI serializes, not a runtime secret holder.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TypeSafeGuardrailConfigModel {
    pub optional_params: Option<TypeSafeOptionalParams>,
    pub api_key: Option<String>,
    pub api_base: Option<String>,
    pub model: Option<String>,
    pub unreachable_fallback: Option<UnreachableFallback>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TypeSafeSettings {
    pub api_base: String,
    pub model: String,
    pub unreachable_fallback: UnreachableFallback,
    pub relevance_threshold: RelevanceThreshold,
    pub min_chars_to_evaluate: MinCharsToEvaluate,
    pub max_result_chars_in_state: MaxResultChars,
}

impl Default for TypeSafeSettings {
    fn default() -> Self {
        Self {
            api_base: DEFAULT_API_BASE.to_owned(),
            model: DEFAULT_MODEL.to_owned(),
            unreachable_fallback: UnreachableFallback::FailOpen,
            relevance_threshold: RelevanceThreshold::default(),
            min_chars_to_evaluate: MinCharsToEvaluate::default(),
            max_result_chars_in_state: MaxResultChars::default(),
        }
    }
}

impl TypeSafeGuardrailConfigModel {
    #[must_use]
    pub fn resolve_settings(&self) -> TypeSafeSettings {
        let optional = self.optional_params.as_ref();
        TypeSafeSettings {
            api_base: self
                .api_base
                .as_deref()
                .unwrap_or(DEFAULT_API_BASE)
                .trim_end_matches('/')
                .to_owned(),
            model: self
                .model
                .clone()
                .unwrap_or_else(|| DEFAULT_MODEL.to_owned()),
            unreachable_fallback: self
                .unreachable_fallback
                .unwrap_or(UnreachableFallback::FailOpen),
            relevance_threshold: optional
                .and_then(|params| params.relevance_threshold)
                .unwrap_or_default(),
            min_chars_to_evaluate: optional
                .and_then(|params| params.min_chars_to_evaluate)
                .unwrap_or_default(),
            max_result_chars_in_state: optional
                .and_then(|params| params.max_result_chars_in_state)
                .unwrap_or_default(),
        }
    }
}

impl GuardrailConfigModelIdentity for TypeSafeGuardrailConfigModel {
    fn ui_friendly_name() -> &'static str {
        "TypeSafe (Jev) Compaction"
    }
}

#[path = "typesafe_config/tests.rs"]
mod tests;
