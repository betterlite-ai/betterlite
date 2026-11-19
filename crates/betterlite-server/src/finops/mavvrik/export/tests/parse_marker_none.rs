#![cfg(test)]

use betterlite_logging::mavvrik_delivery::api::MetricsMarker;
use rstest::rstest;
use tracing::Level;

use crate::finops::mavvrik::export::parse_metrics_marker;
use crate::finops::mavvrik::support::capture_logs;

#[rstest]
#[case::zero(MetricsMarker::Epoch(0))]
#[case::empty(MetricsMarker::Text(String::new()))]
#[case::blank(MetricsMarker::Text(String::from("   ")))]
fn a_falsy_marker_parses_to_none(#[case] marker: MetricsMarker) {
    assert_eq!(parse_metrics_marker(&marker), None);
}

#[test]
fn a_missing_marker_arrives_as_epoch_zero_and_parses_to_none() {
    assert_eq!(MetricsMarker::default(), MetricsMarker::Epoch(0));
    assert_eq!(parse_metrics_marker(&MetricsMarker::default()), None);
}

#[test]
fn a_falsy_marker_is_not_a_parse_failure() {
    let (logs, _guard) = capture_logs();
    assert_eq!(parse_metrics_marker(&MetricsMarker::Epoch(0)), None);
    assert_eq!(
        parse_metrics_marker(&MetricsMarker::Text(String::new())),
        None
    );
    assert_eq!(
        parse_metrics_marker(&MetricsMarker::Text(String::from("  "))),
        None
    );
    assert!(!logs.has_at(Level::WARN, "could not parse"));
}
