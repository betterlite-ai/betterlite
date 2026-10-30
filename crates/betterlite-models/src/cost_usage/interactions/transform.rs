use std::collections::BTreeMap;

use serde_json::value::RawValue;

use super::cache::subtract_cached_from_input;
use super::modality::{ModalityList, modality_token_sums};
use super::record::{InteractionsUsage, field_value};
use super::search::google_search_query_count;
use crate::cost_usage::error::{CostUsageError, ModalityField};
use crate::cost_usage::{CompletionDetails, CostUsage, PromptDetails};

struct ModalitySums {
    input: BTreeMap<ModalityField, i64>,
    output: BTreeMap<ModalityField, i64>,
    total_cached_tokens: i64,
}

const fn has_non_array_modality_list(usage: &InteractionsUsage) -> bool {
    let lists = (
        &usage.input_tokens_by_modality,
        &usage.tool_use_tokens_by_modality,
        &usage.cached_tokens_by_modality,
        &usage.output_tokens_by_modality,
    );
    matches!(
        lists,
        (ModalityList::NotArray, _, _, _)
            | (_, ModalityList::NotArray, _, _)
            | (_, _, ModalityList::NotArray, _)
            | (_, _, _, ModalityList::NotArray)
    )
}

fn compute_modality_sums(usage: &InteractionsUsage) -> Result<ModalitySums, CostUsageError> {
    if has_non_array_modality_list(usage) {
        return Err(CostUsageError::ModalityListNotArray);
    }
    let mut input_entries: Vec<&RawValue> = usage
        .input_tokens_by_modality
        .entries()
        .iter()
        .map(Box::as_ref)
        .collect();
    input_entries.extend(
        usage
            .tool_use_tokens_by_modality
            .entries()
            .iter()
            .map(Box::as_ref),
    );
    let input_sums_raw = modality_token_sums(&input_entries)?;

    let cached_entries: Vec<&RawValue> = usage
        .cached_tokens_by_modality
        .entries()
        .iter()
        .map(Box::as_ref)
        .collect();
    let cached_sums = modality_token_sums(&cached_entries)?;

    let output_entries: Vec<&RawValue> = usage
        .output_tokens_by_modality
        .entries()
        .iter()
        .map(Box::as_ref)
        .collect();
    let output = modality_token_sums(&output_entries)?;

    let total_cached_tokens = usage.total_cached_tokens.value()?;
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

// A falsy total_tokens (including 0) is replaced by the prompt+completion sum.
fn compute_token_totals(usage: &InteractionsUsage) -> Result<TokenTotals, CostUsageError> {
    let reasoning_tokens = usage.total_reasoning_tokens.value()?;
    let reasoning = if reasoning_tokens != 0 {
        reasoning_tokens
    } else {
        usage.total_thought_tokens.value()?
    };

    let prompt = field_value(&usage.total_input_tokens)?
        .checked_add(usage.total_tool_use_tokens.value()?)
        .ok_or(CostUsageError::PromptTotalOverflow)?;

    let completion = field_value(&usage.total_output_tokens)?
        .checked_add(reasoning)
        .ok_or(CostUsageError::CompletionTotalOverflow)?;

    let explicit_total = usage.total_tokens.value()?;
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
    input_sums: &BTreeMap<ModalityField, i64>,
    total_cached_tokens: i64,
    web_search_requests: i64,
) -> Option<PromptDetails> {
    if input_sums.is_empty() && total_cached_tokens == 0 && web_search_requests == 0 {
        return None;
    }
    Some(PromptDetails {
        text_tokens: input_sums.get(&ModalityField::Text).copied(),
        audio_tokens: input_sums.get(&ModalityField::Audio).copied(),
        image_tokens: input_sums.get(&ModalityField::Image).copied(),
        video_tokens: input_sums.get(&ModalityField::Video).copied(),
        web_search_requests: (web_search_requests != 0).then_some(web_search_requests),
    })
}

fn build_completion_details(
    output_sums: &BTreeMap<ModalityField, i64>,
    reasoning_tokens: i64,
) -> Option<CompletionDetails> {
    if output_sums.is_empty() && reasoning_tokens == 0 {
        return None;
    }
    Some(CompletionDetails {
        text_tokens: output_sums.get(&ModalityField::Text).copied(),
        audio_tokens: output_sums.get(&ModalityField::Audio).copied(),
        image_tokens: output_sums.get(&ModalityField::Image).copied(),
        video_tokens: output_sums.get(&ModalityField::Video).copied(),
        reasoning_tokens: (reasoning_tokens != 0).then_some(reasoning_tokens),
    })
}

pub fn transform_interactions_usage(
    usage: &InteractionsUsage,
) -> Result<CostUsage, CostUsageError> {
    let sums = compute_modality_sums(usage)?;
    let totals = compute_token_totals(usage)?;
    let search_entries: Vec<&RawValue> = usage
        .grounding_tool_count
        .entries()
        .iter()
        .map(Box::as_ref)
        .collect();
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
