use std::collections::BTreeMap;

use rust_decimal::prelude::ToPrimitive as _;
use serde::Deserialize;

use super::error::SavingsEvidenceError;
use crate::cost_usage::CompletionDetails;
use crate::price_fields::decimal_lexeme::parse_decimal_lexeme;
use crate::tiered_pricing::Scalar;

// Every numeric leaf keeps its logged shape until coerced below; `extra` only proves the mapping nonempty.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct RawUsageObject {
    pub prompt_tokens: Option<Scalar>,
    pub completion_tokens: Option<Scalar>,
    pub total_tokens: Option<Scalar>,
    pub cache_read_input_tokens: Option<Scalar>,
    pub cache_creation_input_tokens: Option<Scalar>,
    pub prompt_cache_hit_tokens: Option<Scalar>,
    pub completion_tokens_details: Option<RawCompletionTokensDetails>,
    pub prompt_tokens_details: Option<RawPromptTokensDetails>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Scalar>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct RawPromptTokensDetails {
    pub cached_tokens: Option<Scalar>,
    pub cache_creation_tokens: Option<Scalar>,
    pub cache_write_tokens: Option<Scalar>,
    pub text_tokens: Option<Scalar>,
    pub audio_tokens: Option<Scalar>,
    pub image_tokens: Option<Scalar>,
    pub video_tokens: Option<Scalar>,
    pub cache_creation_token_details: Option<Scalar>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct RawCompletionTokensDetails {
    pub reasoning_tokens: Option<Scalar>,
    pub audio_tokens: Option<Scalar>,
    pub text_tokens: Option<Scalar>,
    pub image_tokens: Option<Scalar>,
    pub video_tokens: Option<Scalar>,
}

#[derive(Clone, Debug, Default)]
pub struct LoggedUsage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub cache_read_input_tokens: i64,
    pub cache_creation_input_tokens: i64,
    pub prompt_tokens_details: Option<PromptTokensDetails>,
    pub completion_tokens_details: Option<CompletionDetails>,
}

#[derive(Clone, Debug, Default)]
pub struct PromptTokensDetails {
    pub cached_tokens: i64,
    pub cache_creation_tokens: i64,
    pub cache_write_tokens: i64,
    pub text_tokens: i64,
    pub audio_tokens: i64,
    pub image_tokens: i64,
    pub video_tokens: i64,
    pub cache_creation_token_details: Option<Scalar>,
}

// A malformed usage_object must not fail a spend write; LiteLLM's own guard is just `if not usage_object`.
#[must_use]
pub fn usage_from_spend_log(usage_object: Option<&RawUsageObject>) -> Option<LoggedUsage> {
    let raw = usage_object.filter(|value| {
        value.prompt_tokens.is_some()
            || value.completion_tokens.is_some()
            || value.total_tokens.is_some()
            || value.cache_read_input_tokens.is_some()
            || value.cache_creation_input_tokens.is_some()
            || value.prompt_cache_hit_tokens.is_some()
            || value.completion_tokens_details.is_some()
            || value.prompt_tokens_details.is_some()
            || !value.extra.is_empty()
    })?;
    match coerce_logged_usage(raw) {
        Ok(usage) => Some(usage),
        Err(error) => {
            tracing::warn!(%error, "savings: unusable usage_object, auto-router savings will read zero");
            None
        }
    }
}

fn coerce_logged_usage(raw: &RawUsageObject) -> Result<LoggedUsage, SavingsEvidenceError> {
    let prompt_tokens = numeric_field(raw.prompt_tokens.as_ref())?;
    let completion_tokens = numeric_field(raw.completion_tokens.as_ref())?;
    let total_tokens = numeric_field(raw.total_tokens.as_ref())?;
    let cache_read_input_tokens = numeric_field(raw.cache_read_input_tokens.as_ref())?;
    let cache_creation_input_tokens = numeric_field(raw.cache_creation_input_tokens.as_ref())?;
    let mut prompt_tokens_details = raw
        .prompt_tokens_details
        .as_ref()
        .map(coerce_prompt_tokens_details)
        .transpose()?;
    apply_cache_overrides(raw, &mut prompt_tokens_details)?;
    let completion_tokens_details = raw
        .completion_tokens_details
        .as_ref()
        .map(coerce_completion_details)
        .transpose()?;
    Ok(LoggedUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        cache_read_input_tokens,
        cache_creation_input_tokens,
        prompt_tokens_details,
        completion_tokens_details,
    })
}

// Mirrors Usage.__init__: a top-level cache count always overwrites the matching detail, creating it first.
fn apply_cache_overrides(
    raw: &RawUsageObject,
    details: &mut Option<PromptTokensDetails>,
) -> Result<(), SavingsEvidenceError> {
    if let Some(cached) = integer_like_cache_count(raw.prompt_cache_hit_tokens.as_ref())? {
        details
            .get_or_insert_with(PromptTokensDetails::default)
            .cached_tokens = cached;
    }
    if let Some(cached) = integer_like_cache_count(raw.cache_read_input_tokens.as_ref())? {
        details
            .get_or_insert_with(PromptTokensDetails::default)
            .cached_tokens = cached;
    }
    if let Some(write) = integer_like_cache_count(raw.cache_creation_input_tokens.as_ref())? {
        details
            .get_or_insert_with(PromptTokensDetails::default)
            .cache_write_tokens = write;
    }
    Ok(())
}

fn decimal_to_i64(
    number: &serde_json::Number,
    truncate: bool,
) -> Result<i64, SavingsEvidenceError> {
    let parsed = parse_decimal_lexeme(&number.to_string())
        .map_err(|source| SavingsEvidenceError::AmountOutOfRange { source })?;
    let decimal = if truncate { parsed.trunc() } else { parsed };
    decimal
        .to_i64()
        .ok_or(SavingsEvidenceError::SavedTokensOverflow)
}

// Mirrors Python's isinstance(x, int): a bool or an integer lexeme overrides a detail, a float lexeme never does.
fn integer_like_cache_count(value: Option<&Scalar>) -> Result<Option<i64>, SavingsEvidenceError> {
    match value {
        Some(Scalar::Bool(flag)) => Ok(Some(i64::from(*flag))),
        Some(Scalar::Number(number)) => {
            let lexeme = number.to_string();
            if lexeme.contains(['.', 'e', 'E']) {
                return Ok(None);
            }
            decimal_to_i64(number, false).map(Some)
        }
        _ => Ok(None),
    }
}

fn coerce_prompt_tokens_details(
    raw: &RawPromptTokensDetails,
) -> Result<PromptTokensDetails, SavingsEvidenceError> {
    let cached_tokens = numeric_field(raw.cached_tokens.as_ref())?;
    let cache_creation_tokens = numeric_field(raw.cache_creation_tokens.as_ref())?;
    let cache_write_tokens = numeric_field(raw.cache_write_tokens.as_ref())?;
    let text_tokens = numeric_field(raw.text_tokens.as_ref())?;
    let audio_tokens = numeric_field(raw.audio_tokens.as_ref())?;
    let image_tokens = numeric_field(raw.image_tokens.as_ref())?;
    let video_tokens = numeric_field(raw.video_tokens.as_ref())?;
    Ok(PromptTokensDetails {
        cached_tokens,
        cache_creation_tokens,
        cache_write_tokens,
        text_tokens,
        audio_tokens,
        image_tokens,
        video_tokens,
        cache_creation_token_details: raw.cache_creation_token_details.clone(),
    })
}

fn coerce_completion_details(
    raw: &RawCompletionTokensDetails,
) -> Result<CompletionDetails, SavingsEvidenceError> {
    let text_tokens = optional_numeric_field(raw.text_tokens.as_ref())?;
    let audio_tokens = optional_numeric_field(raw.audio_tokens.as_ref())?;
    let image_tokens = optional_numeric_field(raw.image_tokens.as_ref())?;
    let video_tokens = optional_numeric_field(raw.video_tokens.as_ref())?;
    let reasoning_tokens = optional_numeric_field(raw.reasoning_tokens.as_ref())?;
    Ok(CompletionDetails {
        text_tokens,
        audio_tokens,
        image_tokens,
        video_tokens,
        reasoning_tokens,
    })
}

// A missing key stays absent rather than becoming zero, since callers must tell "unset" from "0" here.
fn optional_numeric_field(value: Option<&Scalar>) -> Result<Option<i64>, SavingsEvidenceError> {
    match value {
        None => Ok(None),
        Some(_) => numeric_field(value).map(Some),
    }
}

// A missing key defaults to zero; a present value that is neither number nor bool fails the whole record.
fn numeric_field(value: Option<&Scalar>) -> Result<i64, SavingsEvidenceError> {
    let Some(scalar) = value else {
        return Ok(0);
    };
    if let Scalar::Bool(flag) = scalar {
        return Ok(i64::from(*flag));
    }
    let Scalar::Number(number) = scalar else {
        return Err(SavingsEvidenceError::NonNumericAmount);
    };
    decimal_to_i64(number, true)
}

pub mod baseline;

mod tests;
