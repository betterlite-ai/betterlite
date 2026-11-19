#![cfg(test)]

use rstest::rstest;

use super::token_buckets;

#[test]
fn a_five_minute_ttl_creates_five_minute_tokens() {
    let buckets = token_buckets(100, 20, 40, 300);
    assert_eq!(buckets.uncached_input_tokens, 20);
    assert_eq!(buckets.cache_read_input_tokens, 40);
    assert_eq!(buckets.cache_creation_5m_input_tokens, 60);
    assert_eq!(buckets.cache_creation_1h_input_tokens, 0);
}

#[test]
fn a_one_hour_ttl_creates_one_hour_tokens() {
    let buckets = token_buckets(100, 20, 40, 3600);
    assert_eq!(buckets.uncached_input_tokens, 20);
    assert_eq!(buckets.cache_read_input_tokens, 40);
    assert_eq!(buckets.cache_creation_5m_input_tokens, 0);
    assert_eq!(buckets.cache_creation_1h_input_tokens, 60);
}

#[rstest]
#[case::zero(0)]
#[case::just_below_five_minutes(299)]
#[case::just_above_five_minutes(301)]
#[case::just_below_one_hour(3599)]
#[case::just_above_one_hour(3601)]
fn an_unmatched_ttl_creates_no_tokens(#[case] ttl: u32) {
    let buckets = token_buckets(100, 20, 40, ttl);
    assert_eq!(buckets.uncached_input_tokens, 20);
    assert_eq!(buckets.cache_read_input_tokens, 40);
    assert_eq!(buckets.cache_creation_5m_input_tokens, 0);
    assert_eq!(buckets.cache_creation_1h_input_tokens, 0);
}

#[test]
fn a_full_read_leaves_no_creation_tokens() {
    let buckets = token_buckets(40, 0, 40, 300);
    assert_eq!(buckets.cache_creation_5m_input_tokens, 0);
}

#[test]
fn a_read_above_the_prefix_saturates_at_zero() {
    let buckets = token_buckets(40, 10, 60, 300);
    assert_eq!(buckets.uncached_input_tokens, 10);
    assert_eq!(buckets.cache_read_input_tokens, 60);
    assert_eq!(buckets.cache_creation_5m_input_tokens, 0);
    assert_eq!(buckets.cache_creation_1h_input_tokens, 0);
}

#[test]
fn no_transfer_creates_the_whole_prefix() {
    let buckets = token_buckets(100, 5, 0, 3600);
    assert_eq!(buckets.cache_creation_1h_input_tokens, 100);
    assert_eq!(buckets.uncached_input_tokens, 5);
}
