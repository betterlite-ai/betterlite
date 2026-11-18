#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::finops::focus::support::{self, ExportCall, Recorder, at};

#[rstest]
#[case::mid_hour(
    "2024-03-10T13:47:23.123456Z",
    "2024-03-10T12:00:00Z",
    "2024-03-10T13:00:00Z"
)]
#[case::on_the_hour("2024-03-10T13:00:00Z", "2024-03-10T12:00:00Z", "2024-03-10T13:00:00Z")]
#[case::just_after_the_hour(
    "2024-03-10T13:00:00.000001Z",
    "2024-03-10T12:00:00Z",
    "2024-03-10T13:00:00Z"
)]
#[case::across_midnight("2024-03-10T00:30:00Z", "2024-03-09T23:00:00Z", "2024-03-10T00:00:00Z")]
fn the_hourly_window_is_the_last_full_hour(
    #[case] now: &str,
    #[case] expected_start: &str,
    #[case] expected_end: &str,
) {
    let window = support::configured("hourly", None, None)
        .compute_time_window(at(now))
        .expect("hourly has a window");
    assert_eq!(window.start_time, at(expected_start));
    assert_eq!(window.end_time, at(expected_end));
    assert_eq!(window.frequency, "hourly");
}

#[tokio::test]
async fn the_scheduled_run_exports_the_last_full_hour() {
    let recorder = Recorder::new(false);
    let logger = support::build(
        &BTreeMap::new(),
        support::params(),
        recorder.factory(),
        None,
    );
    logger
        .initialize_focus_export_job()
        .await
        .expect("the fake export succeeds");
    assert_eq!(
        recorder.calls(),
        vec![ExportCall::Window {
            start: at("2024-03-10T12:00:00Z"),
            end: at("2024-03-10T13:00:00Z"),
            frequency: "hourly".to_owned(),
            limit: None,
        }]
    );
}
