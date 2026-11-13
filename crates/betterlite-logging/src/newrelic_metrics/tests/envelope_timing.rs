use std::time::Duration;

use rstest::rstest;

use super::support::{envelope, record};
use crate::newrelic_metrics::aggregation::build_metric_payload;

#[test]
fn timestamp_is_the_window_start_in_milliseconds() {
    let envelope = envelope(&[record()], 1_700_000_000_123, 1_700_000_001_623);
    assert_eq!(envelope.common.timestamp, 1_700_000_000_123);
}

#[test]
fn interval_is_the_length_of_the_flush_window() {
    let envelope = envelope(&[record()], 1_700_000_000_000, 1_700_000_001_500);
    assert_eq!(envelope.common.interval_ms, 1500);
}

#[rstest]
#[case::empty_window(1_700_000_000_000, 1_700_000_000_000)]
#[case::reversed_window(1_700_000_000_500, 1_700_000_000_000)]
fn interval_has_a_one_millisecond_floor(#[case] window_start_ms: u64, #[case] now_ms: u64) {
    let envelope = envelope(&[record()], window_start_ms, now_ms);
    assert_eq!(envelope.common.interval_ms, 1);
    assert_eq!(envelope.common.timestamp, window_start_ms);
}

#[test]
fn interval_truncates_sub_millisecond_windows() {
    let envelope = build_metric_payload(
        &[record()],
        Duration::new(1_700_000_000, 0),
        Duration::new(1_700_000_000, 1_900_000),
    )
    .expect("an envelope");
    assert_eq!(envelope.common.interval_ms, 1);
}

#[test]
fn timestamp_truncates_sub_millisecond_digits() {
    let envelope = build_metric_payload(
        &[record()],
        Duration::new(1_700_000_000, 123_900_000),
        Duration::new(1_700_000_000, 123_900_000),
    )
    .expect("an envelope");
    assert_eq!(envelope.common.timestamp, 1_700_000_000_123);
}
