#![cfg(test)]

use betterlite_logging::mavvrik_delivery::api::MetricsMarker;

use crate::finops::mavvrik::scheduled_export::run_scheduled_export;
use crate::finops::mavvrik::support::{FIXED_NOW, Recorder, at, build, day, environment};

#[tokio::test]
async fn yesterday_is_the_last_window_and_ends_at_now() {
    let mut recorder = Recorder::default();
    recorder.marker = MetricsMarker::Text(String::from("2024-03-07"));
    let logger = build(&environment(&[]), &recorder, None);
    run_scheduled_export(&logger)
        .await
        .expect("the scheduled run succeeds");
    let windows = recorder.windows();
    let (last, earlier) = windows.split_last().expect("the run exports a window");
    assert_eq!(*last, (day("2024-03-09"), at(FIXED_NOW)));
    assert!(!earlier.is_empty(), "the missed dates come first");
    assert!(
        earlier
            .iter()
            .all(|(start, _end)| *start < day("2024-03-09")),
        "no catch-up window starts at yesterday"
    );
    assert_eq!(
        recorder.frequencies().last().map(String::as_str),
        Some("daily")
    );
}
