#![cfg(test)]

use super::billing_counters;
use crate::cost_usage::error::CostUsageError;
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
    let counts = billing_counters(&usage, None).unwrap();
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
    assert_eq!(billing_counters(&usage, None).unwrap().cache_read, 6);
}

#[rstest::rstest]
#[case::reasoning("reasoning_tokens")]
#[case::cache_read("_cache_read_input_tokens")]
#[case::cache_creation("_cache_creation_input_tokens")]
fn positive_fallback_counter_overflow_is_an_error(#[case] key: &str) {
    let usage = CostUsage {
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
        prompt_details: None,
        completion_details: None,
        cache_read_tokens: None,
        numeric_extras: BTreeMap::from([(key.to_owned(), Decimal::MAX)]),
    };
    assert_eq!(
        billing_counters(&usage, None).unwrap_err(),
        CostUsageError::CounterOverflow
    );
}

#[test]
fn nonzero_reasoning_ignores_overflowing_fallback() {
    let usage = CostUsage {
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
        prompt_details: None,
        cache_read_tokens: None,
        completion_details: Some(CompletionDetails {
            text_tokens: None,
            audio_tokens: None,
            image_tokens: None,
            video_tokens: None,
            reasoning_tokens: Some(7),
        }),
        numeric_extras: BTreeMap::from([("reasoning_tokens".to_owned(), Decimal::MAX)]),
    };
    assert_eq!(billing_counters(&usage, None).unwrap().reasoning, 7);
}
