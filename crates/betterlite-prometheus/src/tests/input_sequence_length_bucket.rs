#![cfg(test)]
use betterlite_prometheus::input_sequence_length::sequence_length_bucket;
use rstest::rstest;

#[rstest]
#[case::noninteger(None, "unknown")]
#[case::negative(Some(-1), "unknown")]
#[case::zero(Some(0), "0-1k")]
#[case::below_1k(Some(999), "0-1k")]
#[case::at_1k(Some(1_000), "1k-4k")]
#[case::below_4k(Some(3_999), "1k-4k")]
#[case::at_4k(Some(4_000), "4k-16k")]
#[case::below_16k(Some(15_999), "4k-16k")]
#[case::at_16k(Some(16_000), "16k-64k")]
#[case::below_64k(Some(63_999), "16k-64k")]
#[case::at_64k(Some(64_000), "64k+")]
#[case::maximum(Some(i64::MAX), "64k+")]
fn handles_boundaries(#[case] input: Option<i64>, #[case] expected: &str) {
    assert_eq!(sequence_length_bucket(input), expected);
}

#[test]
fn duration_boundaries_match() {
    use betterlite_prometheus::buckets::{BATCH_DURATION_BUCKETS, LATENCY_BUCKETS, TimeBound};
    use std::time::Duration;
    let mut latency = [
        5, 10, 25, 50, 100, 250, 500, 1_000, 2_000, 5_000, 10_000, 30_000, 60_000, 120_000,
        300_000, 420_000, 600_000,
    ]
    .map(|millis| TimeBound::Finite(Duration::from_millis(millis)))
    .to_vec();
    latency.push(TimeBound::Infinite);
    assert_eq!(LATENCY_BUCKETS, latency.as_slice());
    let mut batch = [
        60, 120, 300, 600, 900, 1_800, 3_600, 7_200, 14_400, 28_800, 43_200, 86_400,
    ]
    .map(|seconds| TimeBound::Finite(Duration::from_secs(seconds)))
    .to_vec();
    batch.push(TimeBound::Infinite);
    assert_eq!(BATCH_DURATION_BUCKETS, batch.as_slice());
}
