#![cfg(test)]

use betterlite_anthropic::cache_aware_routing::types::CacheTokenBuckets;
use rstest::rstest;

#[test]
fn total_tokens_sums_all_four_buckets() {
    let buckets = CacheTokenBuckets {
        uncached_input_tokens: 1,
        cache_read_input_tokens: 2,
        cache_creation_5m_input_tokens: 3,
        cache_creation_1h_input_tokens: 4,
    };
    assert_eq!(buckets.total_tokens(), 10);
}

#[rstest]
#[case::empty((0, 0, 0, 0), 0)]
#[case::one_bucket((7, 0, 0, 0), 7)]
#[case::two_buckets((7, 0, 3, 0), 10)]
#[case::the_last_bucket((0, 0, 0, 7), 7)]
fn total_tokens_sums_the_present_buckets(
    #[case] counts: (i64, i64, i64, i64),
    #[case] expected: i64,
) {
    let buckets = CacheTokenBuckets {
        uncached_input_tokens: counts.0,
        cache_read_input_tokens: counts.1,
        cache_creation_5m_input_tokens: counts.2,
        cache_creation_1h_input_tokens: counts.3,
    };
    assert_eq!(buckets.total_tokens(), expected);
}

#[test]
fn total_tokens_saturates_instead_of_overflowing() {
    let buckets = CacheTokenBuckets {
        uncached_input_tokens: i64::MAX,
        cache_read_input_tokens: i64::MAX,
        cache_creation_5m_input_tokens: 1,
        cache_creation_1h_input_tokens: 0,
    };
    assert_eq!(buckets.total_tokens(), i64::MAX);
}

#[test]
fn total_tokens_of_default_buckets_is_zero() {
    assert_eq!(CacheTokenBuckets::default().total_tokens(), 0);
}
