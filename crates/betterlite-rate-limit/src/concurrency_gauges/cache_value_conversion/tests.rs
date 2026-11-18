#![cfg(test)]

use rstest::rstest;

use crate::concurrency_gauges::cache::GaugeCacheValue;
use crate::concurrency_gauges::tests::support::registry;
use crate::concurrency_gauges::{GaugeError, GaugeTimes};
use crate::limiter_records::CacheCounterValue;

use super::{gauge_in_flight, whole_number};

const NOW: f64 = 1_700_000_000.0;
const CUTOFF: f64 = 1_699_996_400.0;

fn gauge_times_at_now() -> GaugeTimes {
    GaugeTimes {
        now: NOW,
        cutoff: CUTOFF,
    }
}

#[test]
fn a_missing_cache_value_reads_as_no_requests_in_flight() {
    let in_flight = gauge_in_flight(None, &gauge_times_at_now()).expect("no requests in flight");
    assert_eq!(in_flight, 0);
}

#[test]
fn a_registry_counts_only_the_slots_inside_the_ttl_window() {
    let value = registry(&[
        ("slot-at-the-cutoff", CUTOFF),
        ("slot-expired", 1_699_996_399.0),
        ("slot-fresh", NOW),
    ]);
    let in_flight = gauge_in_flight(Some(&value), &gauge_times_at_now()).expect("a registry count");
    assert_eq!(in_flight, 2);
}

#[rstest]
#[case::integer(CacheCounterValue::Int(3), 3)]
#[case::zero(CacheCounterValue::Int(0), 0)]
#[case::negative_integer(CacheCounterValue::Int(-2), 0)]
#[case::whole_float(CacheCounterValue::Float(4.0), 4)]
#[case::fractional_float(CacheCounterValue::Float(2.7), 2)]
#[case::padded_text(CacheCounterValue::Text(" 6 ".to_owned()), 6)]
#[case::bytes(CacheCounterValue::Bytes(b"8".to_vec()), 8)]
fn an_integer_mirror_reads_as_its_count_never_below_zero(
    #[case] mirror: CacheCounterValue,
    #[case] expected: i64,
) {
    let value = GaugeCacheValue::Counter(mirror);
    let in_flight = gauge_in_flight(Some(&value), &gauge_times_at_now()).expect("a mirror count");
    assert_eq!(in_flight, expected);
}

#[test]
fn a_mirror_that_is_not_a_whole_number_is_an_error() {
    let value = GaugeCacheValue::Counter(CacheCounterValue::Text("soon".to_owned()));
    let error = gauge_in_flight(Some(&value), &gauge_times_at_now())
        .expect_err("a mirror that is not a number");
    assert!(matches!(error, GaugeError::NotWholeNumber { .. }));
}

#[rstest]
#[case::integer(CacheCounterValue::Int(7), 7)]
#[case::whole_float(CacheCounterValue::Float(7.0), 7)]
#[case::negative_float(CacheCounterValue::Float(-7.9), -7)]
#[case::text(CacheCounterValue::Text("7".to_owned()), 7)]
#[case::padded_text(CacheCounterValue::Text(" -3 ".to_owned()), -3)]
#[case::bytes(CacheCounterValue::Bytes(b"7".to_vec()), 7)]
fn a_whole_number_value_reads_as_itself(#[case] value: CacheCounterValue, #[case] expected: i64) {
    let number = whole_number(&value).expect("a whole number");
    assert_eq!(number, expected);
}

#[rstest]
#[case::text(CacheCounterValue::Text("soon".to_owned()))]
#[case::float_nan(CacheCounterValue::Float(f64::NAN))]
#[case::float_beyond_range(CacheCounterValue::Float(1e30))]
#[case::bytes_that_are_not_utf8(CacheCounterValue::Bytes(vec![0xFF]))]
fn a_value_that_is_not_a_whole_number_is_an_error(#[case] value: CacheCounterValue) {
    assert!(matches!(
        whole_number(&value),
        Err(GaugeError::NotWholeNumber { .. })
    ));
}
