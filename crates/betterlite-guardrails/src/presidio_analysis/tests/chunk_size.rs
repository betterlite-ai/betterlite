#![cfg(test)]

use rstest::rstest;

use crate::presidio_analysis::DEFAULT_CHUNK_SIZE_BYTES;
use crate::presidio_analysis::split::coerce_analyze_chunk_size;

#[rstest]
#[case::unset(None, DEFAULT_CHUNK_SIZE_BYTES)]
#[case::zero(Some(0), DEFAULT_CHUNK_SIZE_BYTES)]
#[case::negative(Some(-5), DEFAULT_CHUNK_SIZE_BYTES)]
#[case::one_is_raised_to_four(Some(1), 4)]
#[case::three_is_raised_to_four(Some(3), 4)]
#[case::four_is_kept(Some(4), 4)]
#[case::large_value_is_kept(Some(123_456), 123_456)]
fn chunk_size_coercion_follows_the_litellm_rules(
    #[case] configured: Option<i64>,
    #[case] expected: usize,
) {
    assert_eq!(coerce_analyze_chunk_size(configured), expected);
}
