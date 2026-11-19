#![cfg(test)]

use betterlite_logging::mavvrik_delivery::api::MetricsMarker;
use rstest::rstest;

use crate::finops::mavvrik::export::is_empty_metrics_marker;

#[rstest]
#[case::zero(MetricsMarker::Epoch(0), true)]
#[case::one(MetricsMarker::Epoch(1), false)]
#[case::negative(MetricsMarker::Epoch(-1), false)]
#[case::epoch_now(MetricsMarker::Epoch(1_709_942_400), false)]
#[case::empty_text(MetricsMarker::Text(String::new()), true)]
#[case::blank_text(MetricsMarker::Text(String::from("  ")), true)]
#[case::zero_text(MetricsMarker::Text(String::from("0")), false)]
#[case::date_text(MetricsMarker::Text(String::from("2024-03-09")), false)]
fn emptiness_is_zero_or_blank_text(#[case] marker: MetricsMarker, #[case] expected: bool) {
    assert_eq!(is_empty_metrics_marker(&marker), expected);
}

#[test]
fn a_missing_marker_counts_as_empty() {
    assert!(is_empty_metrics_marker(&MetricsMarker::default()));
}
