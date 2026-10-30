use std::collections::BTreeMap;

use crate::cost_usage::error::{CostUsageError, ModalityField};

// Falls back to subtracting total_cached_tokens from text_tokens only when no modality was recognized.
pub(crate) fn subtract_cached_from_input(
    input_sums: &BTreeMap<ModalityField, i64>,
    cached_sums: &BTreeMap<ModalityField, i64>,
    total_cached_tokens: i64,
) -> Result<BTreeMap<ModalityField, i64>, CostUsageError> {
    if !cached_sums.is_empty() {
        return subtract_per_modality(input_sums, cached_sums);
    }
    if total_cached_tokens != 0
        && let Some(&text_tokens) = input_sums.get(&ModalityField::Text)
    {
        return subtract_total_from_text(input_sums, text_tokens, total_cached_tokens);
    }
    Ok(input_sums.clone())
}

fn subtract_per_modality(
    input_sums: &BTreeMap<ModalityField, i64>,
    cached_sums: &BTreeMap<ModalityField, i64>,
) -> Result<BTreeMap<ModalityField, i64>, CostUsageError> {
    let mut result = BTreeMap::new();
    for (&field, &tokens) in input_sums {
        let cached = cached_sums.get(&field).copied().unwrap_or(0);
        let subtracted = tokens
            .checked_sub(cached)
            .ok_or(CostUsageError::CacheSubtractionOverflow { field })?;
        result.insert(field, subtracted.max(0));
    }
    Ok(result)
}

fn subtract_total_from_text(
    input_sums: &BTreeMap<ModalityField, i64>,
    text_tokens: i64,
    total_cached_tokens: i64,
) -> Result<BTreeMap<ModalityField, i64>, CostUsageError> {
    let mut result = input_sums.clone();
    let subtracted = text_tokens.checked_sub(total_cached_tokens).ok_or(
        CostUsageError::CacheSubtractionOverflow {
            field: ModalityField::Text,
        },
    )?;
    result.insert(ModalityField::Text, subtracted.max(0));
    Ok(result)
}
