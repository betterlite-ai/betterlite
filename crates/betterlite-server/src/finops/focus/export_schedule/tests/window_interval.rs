#![cfg(test)]

use rstest::rstest;

use crate::finops::focus::support::{self, FIXED_NOW, at};

#[rstest]
#[case::ninety_seconds(Some(90), "2024-03-10T13:45:53.123456Z")]
#[case::sixty_seconds(Some(60), "2024-03-10T13:46:23.123456Z")]
#[case::explicit_zero(Some(0), "2024-03-10T13:47:23.123456Z")]
#[case::one_hour(Some(3600), "2024-03-10T12:47:23.123456Z")]
fn the_interval_window_ends_now_and_is_as_wide_as_the_interval(
    #[case] interval_seconds: Option<i64>,
    #[case] expected_start: &str,
) {
    let window = support::configured("interval", None, interval_seconds)
        .compute_time_window(at(FIXED_NOW))
        .expect("interval is valid");
    assert_eq!(window.start_time, at(expected_start));
    assert_eq!(window.end_time, at(FIXED_NOW));
    assert_eq!(window.frequency, "interval");
}

#[test]
fn an_unset_interval_reads_as_sixty_seconds() {
    let window = support::configured("interval", None, None)
        .compute_time_window(at(FIXED_NOW))
        .expect("interval is valid");
    assert_eq!(window.start_time, at("2024-03-10T13:46:23.123456Z"));
    assert_eq!(window.end_time, at("2024-03-10T13:47:23.123456Z"));
}
