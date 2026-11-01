use std::collections::BTreeMap;

use serde::Deserialize;
use thiserror::Error;

use super::{CostUsage, PromptDetails};

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptionUsageError {
    #[error("missing field `seconds`")]
    MissingSeconds,
    #[error("missing field `input_tokens`")]
    MissingInputTokens,
    #[error("missing field `output_tokens`")]
    MissingOutputTokens,
    #[error("missing field `total_tokens`")]
    MissingTotalTokens,
    #[error("missing field `input_token_details`")]
    MissingInputTokenDetails,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(try_from = "RawTranscriptionUsage")]
pub enum TranscriptionUsage {
    Duration { seconds: f64 },
    Tokens(Box<TranscriptionTokens>),
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TranscriptionUsageKind {
    Duration,
    Tokens,
}

// Flat, not `#[serde(tag = "type")]` on the public enum: tagged-enum buffering loses an `arbitrary_precision` number cast to `f64`.
#[derive(Debug, Clone, Deserialize)]
struct RawTranscriptionUsage {
    #[serde(rename = "type")]
    kind: TranscriptionUsageKind,
    seconds: Option<f64>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    total_tokens: Option<i64>,
    input_token_details: Option<TranscriptionTokenDetails>,
}

impl TryFrom<RawTranscriptionUsage> for TranscriptionUsage {
    type Error = TranscriptionUsageError;

    fn try_from(raw: RawTranscriptionUsage) -> Result<Self, Self::Error> {
        match raw.kind {
            TranscriptionUsageKind::Duration => {
                let seconds = raw.seconds.ok_or(TranscriptionUsageError::MissingSeconds)?;
                Ok(Self::Duration { seconds })
            }
            TranscriptionUsageKind::Tokens => {
                let input_tokens = raw
                    .input_tokens
                    .ok_or(TranscriptionUsageError::MissingInputTokens)?;
                let output_tokens = raw
                    .output_tokens
                    .ok_or(TranscriptionUsageError::MissingOutputTokens)?;
                let total_tokens = raw
                    .total_tokens
                    .ok_or(TranscriptionUsageError::MissingTotalTokens)?;
                let input_token_details = raw
                    .input_token_details
                    .ok_or(TranscriptionUsageError::MissingInputTokenDetails)?;
                Ok(Self::Tokens(Box::new(TranscriptionTokens {
                    input_tokens,
                    output_tokens,
                    total_tokens,
                    input_token_details,
                })))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct TranscriptionTokens {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub input_token_details: TranscriptionTokenDetails,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct TranscriptionTokenDetails {
    pub audio_tokens: i64,
    pub text_tokens: i64,
}

// Duration usage carries no token counts, so it transforms to None instead of zeros.
#[must_use]
pub fn transform_transcription_usage(usage: &TranscriptionUsage) -> Option<CostUsage> {
    let TranscriptionUsage::Tokens(tokens) = usage else {
        return None;
    };
    Some(CostUsage {
        prompt_tokens: tokens.input_tokens,
        completion_tokens: tokens.output_tokens,
        total_tokens: tokens.total_tokens,
        prompt_details: Some(PromptDetails {
            text_tokens: Some(tokens.input_token_details.text_tokens),
            audio_tokens: Some(tokens.input_token_details.audio_tokens),
            image_tokens: None,
            video_tokens: None,
            web_search_requests: None,
        }),
        completion_details: None,
        cache_read_tokens: None,
        numeric_extras: BTreeMap::new(),
    })
}

mod tests;
