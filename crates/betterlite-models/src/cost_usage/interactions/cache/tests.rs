#![cfg(test)]

use std::collections::BTreeMap;

use super::subtract_cached_from_input;
use crate::cost_usage::error::CostUsageError;

#[test]
fn per_modality_branch_clamps_at_zero() {
    let input: BTreeMap<&'static str, i64> = BTreeMap::from([("text_tokens", 5)]);
    let cached: BTreeMap<&'static str, i64> = BTreeMap::from([("text_tokens", 8)]);
    let result = subtract_cached_from_input(&input, &cached, 0).expect("checked subtraction");
    assert_eq!(result.get("text_tokens"), Some(&0));
}

#[test]
fn zero_valued_cached_modality_still_selects_per_modality_branch() {
    let input: BTreeMap<&'static str, i64> = BTreeMap::from([("text_tokens", 5)]);
    let cached: BTreeMap<&'static str, i64> = BTreeMap::from([("audio_tokens", 0)]);
    let result =
        subtract_cached_from_input(&input, &cached, 9).expect("per-modality branch, not total");
    assert_eq!(result.get("text_tokens"), Some(&5));
}

#[test]
fn total_cached_tokens_only_reduces_existing_text_tokens() {
    let input: BTreeMap<&'static str, i64> = BTreeMap::from([("text_tokens", 5)]);
    let cached: BTreeMap<&'static str, i64> = BTreeMap::new();
    let result = subtract_cached_from_input(&input, &cached, 2).expect("total branch");
    assert_eq!(result.get("text_tokens"), Some(&3));
}

#[test]
fn total_cached_tokens_without_text_tokens_leaves_input_untouched() {
    let input: BTreeMap<&'static str, i64> = BTreeMap::from([("audio_tokens", 5)]);
    let cached: BTreeMap<&'static str, i64> = BTreeMap::new();
    let result = subtract_cached_from_input(&input, &cached, 2).expect("no text_tokens to reduce");
    assert_eq!(result.get("audio_tokens"), Some(&5));
}

#[test]
fn cache_subtraction_overflow_is_reported() {
    let input: BTreeMap<&'static str, i64> = BTreeMap::from([("text_tokens", i64::MIN)]);
    let cached: BTreeMap<&'static str, i64> = BTreeMap::from([("text_tokens", 1)]);
    let error =
        subtract_cached_from_input(&input, &cached, 0).expect_err("overflowing subtraction");
    assert_eq!(
        error,
        CostUsageError::CacheSubtractionOverflow {
            field: "text_tokens"
        }
    );
}
