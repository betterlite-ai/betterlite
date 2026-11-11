use betterlite_models::decisions::Json;
use reqwest::StatusCode;
use rust_decimal::Decimal;
use serde::Serialize;
use thiserror::Error;

use super::hooks::GuardrailEventHook;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct JudgeBlockDetail {
    pub error: String,
    pub overall_score: f64,
    pub threshold: f64,
    pub verdicts: Json,
}

#[derive(Debug, Error)]
pub enum JudgeError {
    #[error("an llm_as_a_judge guardrail requires a guardrail_name")]
    MissingGuardrailName,
    #[error("an llm_as_a_judge guardrail requires judge_model in litellm_params")]
    MissingJudgeModel,
    #[error("an llm_as_a_judge guardrail requires at least one criterion")]
    MissingCriteria,
    #[error("llm_as_a_judge criterion weights must sum to 100, got {total}")]
    WeightSum { total: Decimal },
    #[error("a criterion weight is not a representable decimal")]
    UnrepresentableWeight,
    #[error("the criterion weight sum overflowed")]
    WeightOverflow,
    #[error("llm_as_a_judge on_failure must be 'block' or 'log', got '{value}'")]
    InvalidOnFailure { value: String },
    #[error("'{value}' is not a guardrail event hook")]
    InvalidEventHook { value: String },
    #[error(
        "the {} event hook is not supported by the llm_as_a_judge guardrail",
        hook.as_str()
    )]
    UnsupportedEventHook { hook: GuardrailEventHook },
    #[error("litellm_params.{key} is not the declared type")]
    InvalidParam {
        key: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the LLM judge rejected the input: score below threshold")]
    Blocked { detail: JudgeBlockDetail },
}

impl JudgeError {
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        if matches!(self, Self::Blocked { .. }) {
            StatusCode::UNPROCESSABLE_ENTITY
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}
