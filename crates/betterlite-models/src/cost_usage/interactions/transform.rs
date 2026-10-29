use std::collections::BTreeMap;

use super::cache::subtract_cached_from_input;
use super::modality::{as_refs, modality_token_sums};
use super::record::{InteractionsUsage, field_value};
use super::search::google_search_query_count;
use crate::cost_usage::error::CostUsageError;
use crate::cost_usage::{CompletionDetails, CostUsage, PromptDetails};

struct ModalitySums {
    input: BTreeMap<&'static str, i64>,
    output: BTreeMap<&'static str, i64>,
    total_cached_tokens: i64,
}

fn compute_modality_sums(usage: &InteractionsUsage) -> Result<ModalitySums, CostUsageError> {
    let mut input_entries = as_refs(usage.input_tokens_by_modality.as_ref());
    input_entries.extend(as_refs(usage.tool_use_tokens_by_modality.as_ref()));
    let input_sums_raw = modality_token_sums(&input_entries)?;

    let cached_entries = as_refs(usage.cached_tokens_by_modality.as_ref());
    let cached_sums = modality_token_sums(&cached_entries)?;

    let output_entries = as_refs(usage.output_tokens_by_modality.as_ref());
    let output = modality_token_sums(&output_entries)?;

    let total_cached_tokens = usage.total_cached_tokens.0;
    let input = subtract_cached_from_input(&input_sums_raw, &cached_sums, total_cached_tokens)?;

    Ok(ModalitySums {
        input,
        output,
        total_cached_tokens,
    })
}

struct TokenTotals {
    prompt: i64,
    completion: i64,
    total: i64,
    reasoning: i64,
}

/// `total_tokens` keeps an explicit, truthy value as-is; a falsy one (including `0`)
/// is replaced by the prompt+completion sum, never both.
fn compute_token_totals(usage: &InteractionsUsage) -> Result<TokenTotals, CostUsageError> {
    let reasoning = if usage.total_reasoning_tokens.0 != 0 {
        usage.total_reasoning_tokens.0
    } else {
        usage.total_thought_tokens.0
    };

    let prompt = field_value(&usage.total_input_tokens)
        .checked_add(usage.total_tool_use_tokens.0)
        .ok_or(CostUsageError::PromptTotalOverflow)?;

    let completion = field_value(&usage.total_output_tokens)
        .checked_add(reasoning)
        .ok_or(CostUsageError::CompletionTotalOverflow)?;

    let explicit_total = usage.total_tokens.0;
    let total = if explicit_total != 0 {
        explicit_total
    } else {
        prompt
            .checked_add(completion)
            .ok_or(CostUsageError::TotalTokensOverflow)?
    };

    Ok(TokenTotals {
        prompt,
        completion,
        total,
        reasoning,
    })
}

fn build_prompt_details(
    input_sums: &BTreeMap<&'static str, i64>,
    total_cached_tokens: i64,
    web_search_requests: i64,
) -> Option<PromptDetails> {
    if input_sums.is_empty() && total_cached_tokens == 0 && web_search_requests == 0 {
        return None;
    }
    Some(PromptDetails {
        text_tokens: input_sums.get("text_tokens").copied(),
        audio_tokens: input_sums.get("audio_tokens").copied(),
        image_tokens: input_sums.get("image_tokens").copied(),
        video_tokens: input_sums.get("video_tokens").copied(),
        web_search_requests: Some(web_search_requests),
    })
}

fn build_completion_details(
    output_sums: &BTreeMap<&'static str, i64>,
    reasoning_tokens: i64,
) -> Option<CompletionDetails> {
    if output_sums.is_empty() && reasoning_tokens == 0 {
        return None;
    }
    Some(CompletionDetails {
        text_tokens: output_sums.get("text_tokens").copied(),
        audio_tokens: output_sums.get("audio_tokens").copied(),
        image_tokens: output_sums.get("image_tokens").copied(),
        video_tokens: output_sums.get("video_tokens").copied(),
        reasoning_tokens: Some(reasoning_tokens),
    })
}

pub fn transform_interactions_usage(
    usage: &InteractionsUsage,
) -> Result<CostUsage, CostUsageError> {
    let sums = compute_modality_sums(usage)?;
    let totals = compute_token_totals(usage)?;
    let search_entries = as_refs(usage.grounding_tool_count.as_ref());
    let web_search_requests = google_search_query_count(&search_entries)?;

    Ok(CostUsage {
        prompt_tokens: totals.prompt,
        completion_tokens: totals.completion,
        total_tokens: totals.total,
        prompt_details: build_prompt_details(
            &sums.input,
            sums.total_cached_tokens,
            web_search_requests,
        ),
        completion_details: build_completion_details(&sums.output, totals.reasoning),
        cache_read_tokens: (sums.total_cached_tokens != 0).then_some(sums.total_cached_tokens),
    })
}

mod tests;
