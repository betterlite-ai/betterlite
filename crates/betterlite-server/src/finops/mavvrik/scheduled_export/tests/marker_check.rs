#![cfg(test)]

use betterlite_logging::mavvrik_delivery::api::MetricsMarker;
use rstest::rstest;

use crate::finops::mavvrik::scheduled_export::run_scheduled_export;
use crate::finops::mavvrik::support::{Recorder, build, day, environment};

#[rstest]
#[case::behind_yesterday(MetricsMarker::Text(String::from("2024-03-07")), 2)]
#[case::at_yesterday(MetricsMarker::Text(String::from("2024-03-09")), 1)]
#[case::today(MetricsMarker::Text(String::from("2024-03-10")), 1)]
#[case::unparseable(MetricsMarker::Text(String::from("unknown")), 1)]
#[case::fresh_connector(MetricsMarker::Epoch(0), 7)]
#[case::zero_text(MetricsMarker::Text(String::from("0")), 1)]
#[tokio::test]
async fn the_marker_decides_whether_missed_dates_are_exported(
    #[case] marker: MetricsMarker,
    #[case] windows: usize,
) {
    let mut recorder = Recorder::default();
    recorder.marker = marker;
    let logger = build(&environment(&[]), &recorder, None);
    run_scheduled_export(&logger)
        .await
        .expect("the scheduled run succeeds");
    assert_eq!(recorder.windows().len(), windows);
}

#[rstest]
#[case::fresh_connector(MetricsMarker::Epoch(0))]
#[case::empty_text(MetricsMarker::Text(String::new()))]
#[case::blank_text(MetricsMarker::Text(String::from("  ")))]
#[tokio::test]
async fn an_empty_marker_always_catches_the_whole_window_up(#[case] marker: MetricsMarker) {
    let mut recorder = Recorder::default();
    recorder.marker = marker;
    let logger = build(&environment(&[]), &recorder, None);
    run_scheduled_export(&logger)
        .await
        .expect("the scheduled run succeeds");
    let first = recorder.windows().first().copied();
    assert_eq!(first, Some((day("2024-03-03"), day("2024-03-04"))));
}
