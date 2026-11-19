#![cfg(test)]

use betterlite_logging::mavvrik_delivery::api::MetricsMarker;
use tracing::Level;

use crate::finops::mavvrik::scheduled_export::run_scheduled_export;
use crate::finops::mavvrik::support::{
    FIXED_NOW, Recorder, at, build, capture_logs, day, environment,
};

#[tokio::test]
async fn every_missed_date_gets_its_own_window_ending_at_its_midnight() {
    let (logs, _guard) = capture_logs();
    let mut recorder = Recorder::default();
    recorder.marker = MetricsMarker::Text(String::from("2024-03-05"));
    let logger = build(&environment(&[]), &recorder, None);
    run_scheduled_export(&logger)
        .await
        .expect("the scheduled run succeeds");
    assert_eq!(
        recorder.windows(),
        vec![
            (day("2024-03-06"), day("2024-03-07")),
            (day("2024-03-07"), day("2024-03-08")),
            (day("2024-03-08"), day("2024-03-09")),
            (day("2024-03-09"), at(FIXED_NOW)),
        ]
    );
    assert!(
        recorder
            .frequencies()
            .iter()
            .all(|frequency| frequency.as_str() == "daily")
    );
    assert!(logs.has_at(Level::INFO, "catching up missed date 2024-03-06"));
    assert!(logs.has_at(Level::INFO, "catching up missed date 2024-03-08"));
    assert!(!logs.has_at(Level::INFO, "catching up missed date 2024-03-09"));
}

#[tokio::test]
async fn a_catch_up_window_never_ends_after_now() {
    let mut recorder = Recorder::default();
    recorder.marker = MetricsMarker::Text(String::from("2024-03-05T12:00:00Z"));
    let logger = build(&environment(&[]), &recorder, None);
    run_scheduled_export(&logger)
        .await
        .expect("the scheduled run succeeds");
    let windows = recorder.windows();
    let (last, earlier) = windows.split_last().expect("the run exports a window");
    assert_eq!(*last, (day("2024-03-09"), at(FIXED_NOW)));
    assert!(earlier.iter().all(|(_start, end)| *end <= at(FIXED_NOW)));
    assert_eq!(
        earlier.first(),
        Some(&(at("2024-03-06T12:00:00Z"), at("2024-03-07T12:00:00Z")))
    );
    assert_eq!(
        earlier.last(),
        Some(&(at("2024-03-08T12:00:00Z"), at("2024-03-09T12:00:00Z")))
    );
}
