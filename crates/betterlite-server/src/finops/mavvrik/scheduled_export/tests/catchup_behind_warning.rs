#![cfg(test)]

use betterlite_logging::mavvrik_delivery::api::MetricsMarker;
use rstest::rstest;
use tracing::Level;

use crate::finops::mavvrik::scheduled_export::run_scheduled_export;
use crate::finops::mavvrik::support::{Recorder, build, capture_logs, environment};

#[rstest]
#[case::far_behind("2024-02-01", true)]
#[case::one_day_past_the_edge("2024-03-01", true)]
#[case::edge_of_the_window("2024-03-02", false)]
#[case::inside_the_window("2024-03-05", false)]
#[tokio::test]
async fn a_marker_before_the_seven_day_window_warns_where_the_export_starts(
    #[case] date: &str,
    #[case] warns: bool,
) {
    let (logs, _guard) = capture_logs();
    let mut recorder = Recorder::default();
    recorder.marker = MetricsMarker::Text(date.to_owned());
    let logger = build(&environment(&[]), &recorder, None);
    run_scheduled_export(&logger)
        .await
        .expect("the scheduled run succeeds");
    assert_eq!(logs.has_at(Level::WARN, "more than 7 days behind"), warns);
    if warns {
        assert!(logs.has_at(Level::WARN, date));
        assert!(logs.has_at(Level::WARN, "Catching up from 2024-03-03 only"));
        assert!(logs.has_at(Level::WARN, "earlier data will not be re-exported"));
    }
}

#[tokio::test]
async fn an_empty_marker_is_not_reported_as_behind() {
    let (logs, _guard) = capture_logs();
    let recorder = Recorder::default();
    let logger = build(&environment(&[]), &recorder, None);
    run_scheduled_export(&logger)
        .await
        .expect("the scheduled run succeeds");
    assert!(!logs.has_at(Level::WARN, "more than 7 days behind"));
}
