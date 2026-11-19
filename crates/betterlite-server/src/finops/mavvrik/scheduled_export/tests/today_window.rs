#![cfg(test)]

use betterlite_logging::mavvrik_delivery::api::MetricsMarker;

use crate::finops::mavvrik::scheduled_export::run_scheduled_export;
use crate::finops::mavvrik::support::{FIXED_NOW, Recorder, at, build, day, environment};

#[tokio::test]
async fn the_daily_window_starts_at_yesterday_midnight_and_ends_now() {
    let mut recorder = Recorder::default();
    recorder.marker = MetricsMarker::Epoch(1_709_942_400);
    let logger = build(&environment(&[]), &recorder, None);
    run_scheduled_export(&logger)
        .await
        .expect("the scheduled run succeeds");
    assert_eq!(recorder.windows(), vec![(day("2024-03-09"), at(FIXED_NOW))]);
    let (start, end) = recorder.windows()[0];
    assert!(start < end);
    assert!(
        end > day("2024-03-10"),
        "the window ends at now, not at today's midnight"
    );
    assert_eq!(recorder.frequencies(), vec![String::from("daily")]);
}
