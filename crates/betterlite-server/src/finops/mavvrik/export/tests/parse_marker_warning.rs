#![cfg(test)]

use betterlite_logging::mavvrik_delivery::api::MetricsMarker;
use rstest::rstest;
use tracing::Level;

use crate::finops::mavvrik::export::parse_metrics_marker;
use crate::finops::mavvrik::support::capture_logs;

#[rstest]
#[case::words("not-a-date")]
#[case::impossible_month("2024-13-45")]
#[case::unknown_day("2024-02-30")]
#[case::time_only("12:34:56")]
fn an_unparseable_text_marker_warns_and_parses_to_none(#[case] text: &str) {
    let (logs, _guard) = capture_logs();
    assert_eq!(
        parse_metrics_marker(&MetricsMarker::Text(text.to_owned())),
        None
    );
    assert!(logs.has_at(Level::WARN, "could not parse metricsMarker"));
}

#[test]
fn an_out_of_range_timestamp_warns_and_parses_to_none() {
    let (logs, _guard) = capture_logs();
    assert_eq!(parse_metrics_marker(&MetricsMarker::Epoch(i64::MAX)), None);
    assert!(logs.has_at(Level::WARN, "could not parse metricsMarker"));
}
