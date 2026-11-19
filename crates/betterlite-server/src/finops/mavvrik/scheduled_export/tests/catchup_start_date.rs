#![cfg(test)]

use betterlite_logging::mavvrik_delivery::api::MetricsMarker;
use rstest::rstest;

use crate::finops::mavvrik::scheduled_export::run_scheduled_export;
use crate::finops::mavvrik::support::{Recorder, build, day, environment};

#[rstest]
#[case::fresh_connector(MetricsMarker::Epoch(0), "2024-03-03")]
#[case::one_day_behind(MetricsMarker::Text(String::from("2024-03-08")), "2024-03-09")]
#[case::two_days_behind(MetricsMarker::Text(String::from("2024-03-07")), "2024-03-08")]
#[case::inside_the_window(MetricsMarker::Text(String::from("2024-03-05")), "2024-03-06")]
#[case::at_the_window_edge(MetricsMarker::Text(String::from("2024-03-02")), "2024-03-03")]
#[case::before_the_window(MetricsMarker::Text(String::from("2024-02-01")), "2024-03-03")]
#[tokio::test]
async fn the_first_exported_date_is_the_day_after_the_marker_or_the_window_start(
    #[case] marker: MetricsMarker,
    #[case] first: &str,
) {
    let mut recorder = Recorder::new();
    recorder.marker = marker;
    let logger = build(&environment(&[]), &recorder, None);
    run_scheduled_export(&logger)
        .await
        .expect("the scheduled run succeeds");
    let windows = recorder.windows();
    assert_eq!(
        windows.first().map(|(start, _end)| *start),
        Some(day(first))
    );
}

#[tokio::test]
async fn a_fresh_connector_exports_the_full_seven_day_window() {
    let recorder = Recorder::default();
    let logger = build(&environment(&[]), &recorder, None);
    run_scheduled_export(&logger)
        .await
        .expect("the scheduled run succeeds");
    let starts: Vec<_> = recorder
        .windows()
        .into_iter()
        .map(|(start, _end)| start)
        .collect();
    assert_eq!(
        starts,
        vec![
            day("2024-03-03"),
            day("2024-03-04"),
            day("2024-03-05"),
            day("2024-03-06"),
            day("2024-03-07"),
            day("2024-03-08"),
            day("2024-03-09"),
        ]
    );
}
