use rust_decimal::Decimal;

use super::error::TokenCostError;
use crate::cost_usage::CostUsage;
use crate::cost_usage::modality_projection::{
    CacheCreationTokenDetails, CachedTokenCounters, PromptModalityUsage,
};
use crate::modality_cost::prompt::{PromptTokensDetailsResult, parse_prompt_tokens_details};

// The counters PromptModalityUsage needs beyond CostUsage's own prompt_details and cache_read_tokens.
#[derive(Debug, Clone, Copy, Default)]
pub struct PromptExtras {
    pub cache_write_tokens: Option<i64>,
    pub cache_creation_tokens: Option<i64>,
    pub cache_creation_token_details: Option<CacheCreationTokenDetails>,
    pub cached_tokens_details: Option<CachedTokenCounters>,
    pub character_count: Option<i64>,
    pub image_count: Option<i64>,
    pub video_length_seconds: Option<Decimal>,
    pub audio_length_seconds: Option<Decimal>,
    pub query_count: Option<i64>,
}

// Prompt usage begins as all text when prompt_details is absent; otherwise it uses B1671's parsed details.
pub fn prompt_tokens_details(
    usage: &CostUsage,
    extras: Option<&PromptExtras>,
) -> Result<PromptTokensDetailsResult, TokenCostError> {
    let Some(base) = usage.prompt_details else {
        return Ok(PromptTokensDetailsResult {
            cache_hit_tokens: 0,
            cache_hit_audio_tokens: 0,
            cache_creation_tokens: 0,
            cache_creation_token_details: None,
            text_tokens: usage.prompt_tokens,
            audio_tokens: 0,
            image_tokens: 0,
            video_tokens: 0,
            character_count: 0,
            image_count: 0,
            video_length_seconds: Decimal::ZERO,
            audio_length_seconds: Decimal::ZERO,
            query_count: 0,
        });
    };
    let modality_usage = PromptModalityUsage {
        base,
        cached_tokens: usage.cache_read_tokens,
        cache_write_tokens: extras.and_then(|fields| fields.cache_write_tokens),
        cache_creation_tokens: extras.and_then(|fields| fields.cache_creation_tokens),
        cache_creation_token_details: extras.and_then(|fields| fields.cache_creation_token_details),
        cached_tokens_details: extras.and_then(|fields| fields.cached_tokens_details),
        character_count: extras.and_then(|fields| fields.character_count),
        image_count: extras.and_then(|fields| fields.image_count),
        video_length_seconds: extras.and_then(|fields| fields.video_length_seconds),
        audio_length_seconds: extras.and_then(|fields| fields.audio_length_seconds),
        query_count: extras.and_then(|fields| fields.query_count),
    };
    Ok(parse_prompt_tokens_details(&modality_usage)?)
}

fn allocate_uncached_modalities(
    mut details: PromptTokensDetailsResult,
    prompt_tokens: i64,
) -> Result<PromptTokensDetailsResult, TokenCostError> {
    let overflow = || TokenCostError::Overflow;
    let uncached_budget = prompt_tokens
        .checked_sub(details.cache_hit_tokens)
        .and_then(|value| value.checked_sub(details.cache_creation_tokens))
        .ok_or_else(overflow)?
        .max(0);
    let billable_audio = details.audio_tokens.min(uncached_budget);
    let billable_image = details.image_tokens.min(
        uncached_budget
            .checked_sub(billable_audio)
            .ok_or_else(overflow)?,
    );
    let billable_video = details.video_tokens.min(
        uncached_budget
            .checked_sub(billable_audio)
            .and_then(|value| value.checked_sub(billable_image))
            .ok_or_else(overflow)?,
    );
    details.audio_tokens = billable_audio;
    details.image_tokens = billable_image;
    details.video_tokens = billable_video;
    details.text_tokens = uncached_budget
        .checked_sub(billable_audio)
        .and_then(|value| value.checked_sub(billable_image))
        .and_then(|value| value.checked_sub(billable_video))
        .ok_or_else(overflow)?;
    Ok(details)
}

fn infer_text_residual(
    mut details: PromptTokensDetailsResult,
    prompt_tokens: i64,
) -> Result<PromptTokensDetailsResult, TokenCostError> {
    let overflow = || TokenCostError::Overflow;
    if details.text_tokens == 0 && details.image_count == 0 {
        details.text_tokens = prompt_tokens
            .checked_sub(details.cache_hit_tokens)
            .and_then(|value| value.checked_sub(details.audio_tokens))
            .and_then(|value| value.checked_sub(details.cache_creation_tokens))
            .and_then(|value| value.checked_sub(details.image_tokens))
            .and_then(|value| value.checked_sub(details.video_tokens))
            .ok_or_else(overflow)?
            .max(0);
    }
    Ok(details)
}

// Constrains noncached audio/image/video to the cache-free budget when they overlap the cache; otherwise infers a zero text as the residual unless image_count blocks it.
pub fn repair_prompt_overlap(
    details: PromptTokensDetailsResult,
    prompt_tokens: i64,
) -> Result<PromptTokensDetailsResult, TokenCostError> {
    let overflow = || TokenCostError::Overflow;
    let total_details = details
        .text_tokens
        .checked_add(details.cache_hit_tokens)
        .and_then(|sum| sum.checked_add(details.audio_tokens))
        .and_then(|sum| sum.checked_add(details.cache_creation_tokens))
        .and_then(|sum| sum.checked_add(details.image_tokens))
        .and_then(|sum| sum.checked_add(details.video_tokens))
        .ok_or_else(overflow)?;
    let has_double_counting = (details.cache_hit_tokens > 0 || details.cache_creation_tokens > 0)
        && total_details > prompt_tokens;

    if has_double_counting {
        allocate_uncached_modalities(details, prompt_tokens)
    } else {
        infer_text_residual(details, prompt_tokens)
    }
}

mod tests;
