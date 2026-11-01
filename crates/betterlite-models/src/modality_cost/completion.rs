use super::error::ModalityCostError;
use crate::cost_usage::CompletionDetails;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompletionTokensDetailsResult {
    pub audio_tokens: i64,
    pub text_tokens: i64,
    pub reasoning_tokens: i64,
    pub image_tokens: i64,
    pub video_tokens: i64,
}

// Subtracts the minimum of reasoning, text, and the reported total's excess over completion_tokens.
fn text_without_nested_reasoning(
    completion_tokens: i64,
    text_tokens: i64,
    reasoning_tokens: i64,
    other_modality_tokens: i64,
) -> Result<i64, ModalityCostError> {
    let overflow = || ModalityCostError::ReasoningRemovalOverflow;
    let reported_total = text_tokens
        .checked_add(reasoning_tokens)
        .and_then(|sum| sum.checked_add(other_modality_tokens))
        .ok_or_else(overflow)?;
    let excess = reported_total
        .checked_sub(completion_tokens)
        .ok_or_else(overflow)?
        .max(0);
    let nested_reasoning = reasoning_tokens.min(text_tokens).min(excess);
    text_tokens
        .checked_sub(nested_reasoning)
        .ok_or_else(overflow)
}

pub fn parse_completion_details(
    completion_tokens: i64,
    usage: &CompletionDetails,
) -> Result<CompletionTokensDetailsResult, ModalityCostError> {
    let audio_tokens = usage.audio_tokens.unwrap_or(0);
    let reasoning_tokens = usage.reasoning_tokens.unwrap_or(0);
    let image_tokens = usage.image_tokens.unwrap_or(0);
    let video_tokens = usage.video_tokens.unwrap_or(0).max(0);
    let other_modality_tokens = audio_tokens
        .checked_add(image_tokens)
        .and_then(|sum| sum.checked_add(video_tokens))
        .ok_or(ModalityCostError::ReasoningRemovalOverflow)?;

    let text_tokens = text_without_nested_reasoning(
        completion_tokens,
        usage.text_tokens.unwrap_or(0),
        reasoning_tokens,
        other_modality_tokens,
    )?;

    Ok(CompletionTokensDetailsResult {
        audio_tokens,
        text_tokens,
        reasoning_tokens,
        image_tokens,
        video_tokens,
    })
}

mod tests;
