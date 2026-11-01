use rust_decimal::Decimal;

use super::error::ModalityCostError;
use crate::cost_usage::modality_projection::{
    CacheCreationTokenDetails, PromptModalityUsage, resolved_cache_creation_tokens,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptTokensDetailsResult {
    pub cache_hit_tokens: i64,
    pub cache_hit_audio_tokens: i64,
    pub cache_creation_tokens: i64,
    pub cache_creation_token_details: Option<CacheCreationTokenDetails>,
    pub text_tokens: i64,
    pub audio_tokens: i64,
    pub image_tokens: i64,
    pub video_tokens: i64,
    pub character_count: i64,
    pub image_count: i64,
    pub video_length_seconds: Decimal,
    pub audio_length_seconds: Decimal,
    pub query_count: i64,
}

// Allocates the reported cache hit to audio, then text, then image, each capped by what remains.
fn allocate_cache(
    usage: &PromptModalityUsage,
    cache_hit_tokens: i64,
) -> Result<(i64, i64, i64), ModalityCostError> {
    let overflow = || ModalityCostError::CacheAllocationOverflow;
    let details = usage.cached_tokens_details.unwrap_or_default();
    let cached_audio = details.audio_tokens.unwrap_or(0).min(cache_hit_tokens);
    let remaining_after_audio = cache_hit_tokens
        .checked_sub(cached_audio)
        .ok_or_else(overflow)?;
    let cached_text = details.text_tokens.unwrap_or(0).min(remaining_after_audio);
    let remaining_after_text = remaining_after_audio
        .checked_sub(cached_text)
        .ok_or_else(overflow)?;
    let cached_image = details.image_tokens.unwrap_or(0).min(remaining_after_text);
    Ok((cached_audio, cached_text, cached_image))
}

pub fn parse_prompt_tokens_details(
    usage: &PromptModalityUsage,
) -> Result<PromptTokensDetailsResult, ModalityCostError> {
    let overflow = || ModalityCostError::CacheAllocationOverflow;
    let cache_hit_tokens = usage.cached_tokens.unwrap_or(0);
    let (cached_audio, cached_text, cached_image) = allocate_cache(usage, cache_hit_tokens)?;

    let text_tokens = usage
        .base
        .text_tokens
        .unwrap_or(0)
        .checked_sub(cached_text)
        .ok_or_else(overflow)?
        .max(0);
    let audio_tokens = usage
        .base
        .audio_tokens
        .unwrap_or(0)
        .checked_sub(cached_audio)
        .ok_or_else(overflow)?
        .max(0);
    let image_tokens = usage
        .base
        .image_tokens
        .unwrap_or(0)
        .checked_sub(cached_image)
        .ok_or_else(overflow)?
        .max(0);

    Ok(PromptTokensDetailsResult {
        cache_hit_tokens,
        cache_hit_audio_tokens: cached_audio,
        cache_creation_tokens: resolved_cache_creation_tokens(usage),
        cache_creation_token_details: usage.cache_creation_token_details,
        text_tokens,
        audio_tokens,
        image_tokens,
        video_tokens: usage.base.video_tokens.unwrap_or(0).max(0),
        character_count: usage.character_count.unwrap_or(0),
        image_count: usage.image_count.unwrap_or(0),
        video_length_seconds: usage.video_length_seconds.unwrap_or(Decimal::ZERO),
        audio_length_seconds: usage.audio_length_seconds.unwrap_or(Decimal::ZERO),
        query_count: usage.query_count.unwrap_or(0).max(0),
    })
}

// Subtracts cache-hit tokens from prompt_tokens without clamping at zero or touching cache writes.
pub fn get_billable_input_tokens(
    prompt_tokens: i64,
    usage: &PromptModalityUsage,
) -> Result<i64, ModalityCostError> {
    let details = parse_prompt_tokens_details(usage)?;
    prompt_tokens
        .checked_sub(details.cache_hit_tokens)
        .ok_or(ModalityCostError::CacheAllocationOverflow)
}

mod tests;
