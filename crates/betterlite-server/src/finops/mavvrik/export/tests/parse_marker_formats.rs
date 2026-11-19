#![cfg(test)]

use betterlite_logging::mavvrik_delivery::api::MetricsMarker;
use rstest::rstest;

use crate::finops::mavvrik::export::parse_metrics_marker;
use crate::finops::mavvrik::support::at;

#[rstest]
#[case::start_of_day(1_709_942_400, "2024-03-09T00:00:00Z")]
#[case::within_the_day(1_709_949_199, "2024-03-09T00:00:00Z")]
#[case::before_the_epoch(-1, "1969-12-31T00:00:00Z")]
fn a_unix_timestamp_becomes_utc_midnight(#[case] seconds: i64, #[case] expected: &str) {
    let parsed = parse_metrics_marker(&MetricsMarker::Epoch(seconds))
        .expect("the test timestamp is inside the calendar");
    assert_eq!(parsed, at(expected));
}

#[rstest]
#[case::date("2024-03-09", "2024-03-09T00:00:00Z")]
#[case::padding(" 2024-03-09 ", "2024-03-09T00:00:00Z")]
#[case::zulu("2024-03-09T12:34:56Z", "2024-03-09T12:34:56Z")]
#[case::no_zone("2024-03-09T12:34:56", "2024-03-09T12:34:56Z")]
fn an_iso_string_is_read_as_utc(#[case] text: &str, #[case] expected: &str) {
    let parsed = parse_metrics_marker(&MetricsMarker::Text(text.to_owned()))
        .expect("the test string is an ISO date");
    assert_eq!(parsed, at(expected));
}

#[test]
fn a_full_iso_marker_keeps_the_time_of_day_it_names() {
    let parsed = parse_metrics_marker(&MetricsMarker::Text(String::from("2024-03-09T12:34:56Z")))
        .expect("the string is an ISO datetime");
    assert_ne!(parsed, at("2024-03-09T00:00:00Z"));
    assert_eq!(parsed.format("%H:%M:%S").to_string(), "12:34:56");
}
