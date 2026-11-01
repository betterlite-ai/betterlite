use std::collections::BTreeMap;

use rust_decimal::prelude::ToPrimitive as _;
use serde::Deserialize;

use super::error::SavingsEvidenceError;
use crate::price_fields::decimal_lexeme::parse_decimal_lexeme;
use crate::tiered_pricing::Scalar;
use crate::token_rates::ModelInfo;

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
    pub completion_tokens_details: Option<Scalar>,
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

#[derive(Clone, Debug, Default)]
pub struct LoggedUsage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub cache_read_input_tokens: i64,
    pub cache_creation_input_tokens: i64,
    pub prompt_tokens_details: Option<PromptTokensDetails>,
    pub completion_tokens_details: Option<Scalar>,
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
    Ok(LoggedUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        cache_read_input_tokens,
        cache_creation_input_tokens,
        prompt_tokens_details,
        completion_tokens_details: raw.completion_tokens_details.clone(),
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

// Mirrors Python's isinstance(x, int): a bool or an integer lexeme overrides a detail, a float lexeme never does.
fn integer_like_cache_count(value: Option<&Scalar>) -> Result<Option<i64>, SavingsEvidenceError> {
    match value {
        Some(Scalar::Bool(flag)) => Ok(Some(i64::from(*flag))),
        Some(Scalar::Number(number)) => {
            let lexeme = number.to_string();
            if lexeme.contains(['.', 'e', 'E']) {
                return Ok(None);
            }
            let decimal = parse_decimal_lexeme(&lexeme)
                .map_err(|source| SavingsEvidenceError::AmountOutOfRange { source })?;
            decimal
                .to_i64()
                .map(Some)
                .ok_or(SavingsEvidenceError::SavedTokensOverflow)
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

// A missing key defaults to zero; a present value that is neither number nor bool fails the whole record.
fn numeric_field(value: Option<&Scalar>) -> Result<i64, SavingsEvidenceError> {
    match value {
        None => Ok(0),
        Some(Scalar::Bool(flag)) => Ok(i64::from(*flag)),
        Some(Scalar::Number(number)) => {
            let decimal = parse_decimal_lexeme(&number.to_string())
                .map_err(|source| SavingsEvidenceError::AmountOutOfRange { source })?;
            decimal
                .trunc()
                .to_i64()
                .ok_or(SavingsEvidenceError::SavedTokensOverflow)
        }
        _ => Err(SavingsEvidenceError::NonNumericAmount),
    }
}

// Unlike the logged extractors, this typed split prefers cache_creation_tokens over cache_write_tokens.
#[must_use]
pub const fn cache_token_split(usage: &LoggedUsage) -> (i64, i64) {
    let Some(details) = &usage.prompt_tokens_details else {
        return (0, 0);
    };
    let created = if details.cache_creation_tokens != 0 {
        details.cache_creation_tokens
    } else {
        details.cache_write_tokens
    };
    (details.cached_tokens, created)
}

// An absent baseline model keeps both buckets priced; a model with no rate key converts them below instead.
#[must_use]
pub fn baseline_cache_rate_keys(baseline_info: Option<&ModelInfo>) -> (bool, bool) {
    let Some(info) = baseline_info else {
        return (true, true);
    };
    (
        scalar_is_nonzero(info.get("cache_read_input_token_cost")),
        scalar_is_nonzero(info.get("cache_creation_input_token_cost")),
    )
}

fn scalar_is_nonzero(scalar: Option<&Scalar>) -> bool {
    match scalar {
        None | Some(Scalar::Null) => false,
        Some(Scalar::Bool(flag)) => *flag,
        Some(Scalar::Number(number)) => {
            parse_decimal_lexeme(&number.to_string()).map_or(true, |decimal| !decimal.is_zero())
        }
        Some(Scalar::Text(text)) => !text.is_empty(),
        Some(Scalar::Array(items)) => !items.is_empty(),
        Some(Scalar::Object(_)) => true,
    }
}

// Converts an unpriced cache bucket to ordinary input; leaves usage untouched when nothing changes.
#[must_use]
pub fn baseline_usage(usage: LoggedUsage, baseline_info: Option<&ModelInfo>) -> LoggedUsage {
    let (cache_read, cache_creation) = cache_token_split(&usage);
    if cache_read <= 0 && cache_creation <= 0 {
        return usage;
    }
    let Some(details) = usage.prompt_tokens_details.clone() else {
        return usage;
    };
    let (prices_reads, prices_writes) = baseline_cache_rate_keys(baseline_info);
    let reads = if prices_reads { cache_read } else { 0 };
    let writes = if prices_writes { cache_creation } else { 0 };
    if reads == cache_read && writes == cache_creation {
        return usage;
    }
    rebuild_baseline_usage(&usage, &details, reads, writes)
}

fn rebuild_baseline_usage(
    usage: &LoggedUsage,
    details: &PromptTokensDetails,
    reads: i64,
    writes: i64,
) -> LoggedUsage {
    let other_modalities = details
        .audio_tokens
        .saturating_add(details.image_tokens)
        .saturating_add(details.video_tokens);
    let text_tokens = usage
        .prompt_tokens
        .saturating_sub(reads)
        .saturating_sub(writes)
        .saturating_sub(other_modalities)
        .max(0);
    let cache_creation_token_details = if writes != 0 {
        details.cache_creation_token_details.clone()
    } else {
        None
    };
    LoggedUsage {
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        total_tokens: usage.total_tokens,
        cache_read_input_tokens: reads,
        cache_creation_input_tokens: writes,
        prompt_tokens_details: Some(PromptTokensDetails {
            cached_tokens: reads,
            cache_creation_tokens: writes,
            cache_write_tokens: writes,
            text_tokens,
            audio_tokens: details.audio_tokens,
            image_tokens: details.image_tokens,
            video_tokens: details.video_tokens,
            cache_creation_token_details,
        }),
        completion_tokens_details: usage.completion_tokens_details.clone(),
    }
}

mod tests;
