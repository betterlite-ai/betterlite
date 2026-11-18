#![cfg(test)]

use rstest::rstest;

use crate::finops::focus::export_schedule::compute_time_window;
use crate::finops::focus::support::{self, at};

#[rstest]
#[case::mid_day(
    "2024-03-10T13:47:23.123456Z",
    "2024-03-09T00:00:00Z",
    "2024-03-10T00:00:00Z"
)]
#[case::exactly_midnight("2024-03-10T00:00:00Z", "2024-03-09T00:00:00Z", "2024-03-10T00:00:00Z")]
#[case::new_year("2024-01-01T05:00:00Z", "2023-12-31T00:00:00Z", "2024-01-01T00:00:00Z")]
#[case::leap_day("2024-03-01T00:00:00Z", "2024-02-29T00:00:00Z", "2024-03-01T00:00:00Z")]
fn the_daily_window_is_the_last_full_day(
    #[case] now: &str,
    #[case] expected_start: &str,
    #[case] expected_end: &str,
) {
    let window = compute_time_window(&support::configured("daily", None, None), at(now))
        .expect("daily has a window");
    assert_eq!(window.start_time, at(expected_start));
    assert_eq!(window.end_time, at(expected_end));
    assert_eq!(window.frequency, "daily");
}
