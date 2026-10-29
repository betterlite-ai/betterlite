use std::collections::BTreeMap;

use crate::cost_usage::error::CostUsageError;

/// If any cached modality was recognized, even with a zero sum, each existing input
/// modality is reduced by its matching cached count and clamped at zero. Otherwise a
/// truthy `total_cached_tokens` is subtracted only from an existing `text_tokens`
/// field; neither branch creates a modality the input sums did not already have.
pub(crate) fn subtract_cached_from_input(
    input_sums: &BTreeMap<&'static str, i64>,
    cached_sums: &BTreeMap<&'static str, i64>,
    total_cached_tokens: i64,
) -> Result<BTreeMap<&'static str, i64>, CostUsageError> {
    if !cached_sums.is_empty() {
        return subtract_per_modality(input_sums, cached_sums);
    }
    if total_cached_tokens != 0
        && let Some(&text_tokens) = input_sums.get("text_tokens")
    {
        return subtract_total_from_text(input_sums, text_tokens, total_cached_tokens);
    }
    Ok(input_sums.clone())
}

fn subtract_per_modality(
    input_sums: &BTreeMap<&'static str, i64>,
    cached_sums: &BTreeMap<&'static str, i64>,
) -> Result<BTreeMap<&'static str, i64>, CostUsageError> {
    let mut result = BTreeMap::new();
    for (&field, &tokens) in input_sums {
        let cached = cached_sums.get(field).copied().unwrap_or(0);
        let subtracted = tokens
            .checked_sub(cached)
            .ok_or(CostUsageError::CacheSubtractionOverflow { field })?;
        result.insert(field, subtracted.max(0));
    }
    Ok(result)
}

fn subtract_total_from_text(
    input_sums: &BTreeMap<&'static str, i64>,
    text_tokens: i64,
    total_cached_tokens: i64,
) -> Result<BTreeMap<&'static str, i64>, CostUsageError> {
    let mut result = input_sums.clone();
    let subtracted = text_tokens.checked_sub(total_cached_tokens).ok_or(
        CostUsageError::CacheSubtractionOverflow {
            field: "text_tokens",
        },
    )?;
    result.insert("text_tokens", subtracted.max(0));
    Ok(result)
}

mod tests;
