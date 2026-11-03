#![cfg(test)]

use super::billing_counters;
use crate::cost_usage::{CompletionDetails, CostUsage};
use rust_decimal::Decimal;
use std::collections::BTreeMap;

#[test]
fn positive_integer_fallback_excludes_negative_and_fractional_counts() {
    let mut usage = CostUsage {
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
        prompt_details: None,
        completion_details: Some(CompletionDetails::default()),
        cache_read_tokens: None,
        numeric_extras: BTreeMap::new(),
    };
    usage
        .numeric_extras
        .insert("reasoning_tokens".to_owned(), Decimal::from(7_u32));
    usage
        .numeric_extras
        .insert("_cache_read_input_tokens".to_owned(), Decimal::from(-8_i32));
    usage.numeric_extras.insert(
        "_cache_creation_input_tokens".to_owned(),
        Decimal::new(15, 1),
    );
    let counts = billing_counters(&usage, None);
    assert_eq!(counts.reasoning, 7);
    assert_eq!(counts.cache_read, 0);
    assert_eq!(counts.cache_creation, 0);
}

#[test]
fn cache_read_bridge_survives_without_details() {
    let usage = CostUsage {
        prompt_tokens: 10,
        completion_tokens: 0,
        total_tokens: 10,
        prompt_details: None,
        completion_details: None,
        cache_read_tokens: Some(6),
        numeric_extras: BTreeMap::new(),
    };
    assert_eq!(billing_counters(&usage, None).cache_read, 6);
}
