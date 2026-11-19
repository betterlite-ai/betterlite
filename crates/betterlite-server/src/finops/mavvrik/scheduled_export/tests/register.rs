#![cfg(test)]

use betterlite_logging::mavvrik_delivery::api::MetricsMarker;

use crate::finops::mavvrik::scheduled_export::run_scheduled_export;
use crate::finops::mavvrik::support::{EngineCall, Recorder, build, environment};

#[tokio::test]
async fn the_run_reads_the_marker_from_the_destination_before_any_window() {
    let mut recorder = Recorder::default();
    recorder.marker = MetricsMarker::Epoch(1_709_942_400);
    let logger = build(&environment(&[]), &recorder, None);
    run_scheduled_export(&logger)
        .await
        .expect("the scheduled run succeeds");
    let calls = recorder.calls();
    assert_eq!(calls.first(), Some(&EngineCall::Marker));
    assert_eq!(
        calls
            .iter()
            .filter(|call| matches!(call, EngineCall::Marker))
            .count(),
        1
    );
    assert!(
        calls
            .iter()
            .any(|call| matches!(call, EngineCall::Rows { .. }))
    );
}

#[tokio::test]
async fn a_failing_marker_read_stops_the_run_before_any_window() {
    let mut recorder = Recorder::default();
    recorder.fail = true;
    let logger = build(&environment(&[]), &recorder, None);
    run_scheduled_export(&logger)
        .await
        .expect_err("the marker read fails the run");
    assert_eq!(recorder.calls(), vec![EngineCall::Marker]);
    assert!(recorder.windows().is_empty());
}
