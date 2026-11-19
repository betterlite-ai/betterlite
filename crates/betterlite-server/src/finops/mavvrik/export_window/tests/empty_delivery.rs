#![cfg(test)]

use betterlite_logging::focus_records::destinations::FocusTimeWindow;
use tracing::Level;

use crate::finops::mavvrik::support::{
    EngineCall, Recorder, build, capture_logs, day, environment, usage_rows,
};

fn window() -> FocusTimeWindow {
    FocusTimeWindow {
        start_time: day("2024-03-09"),
        end_time: day("2024-03-10"),
        frequency: String::from("daily"),
    }
}

#[tokio::test]
async fn a_day_without_rows_is_still_delivered_empty() {
    let (logs, _guard) = capture_logs();
    let recorder = Recorder::default();
    let logger = build(&environment(&[]), &recorder, None);
    logger
        .export_window(&window(), None)
        .await
        .expect("the fake export succeeds");
    assert_eq!(recorder.delivered(), vec![Vec::new()]);
    assert_eq!(
        recorder.windows(),
        vec![(day("2024-03-09"), day("2024-03-10"))]
    );
    assert_eq!(recorder.frequencies(), vec![String::from("daily")]);
    assert!(
        !recorder
            .calls()
            .iter()
            .any(|call| matches!(call, EngineCall::Payload { .. }))
    );
    assert!(logs.has_at(Level::DEBUG, "no usage data for window"));
}

#[tokio::test]
async fn rows_that_normalize_to_nothing_are_delivered_empty() {
    let (logs, _guard) = capture_logs();
    let mut recorder = Recorder::default();
    recorder.rows = usage_rows(2);
    recorder.payload = Some(Vec::new());
    let logger = build(&environment(&[]), &recorder, None);
    logger
        .export_window(&window(), None)
        .await
        .expect("the fake export succeeds");
    assert_eq!(recorder.delivered(), vec![Vec::new()]);
    assert!(
        recorder
            .calls()
            .iter()
            .any(|call| matches!(call, EngineCall::Payload { rows } if *rows == 2))
    );
    assert!(!logs.has_at(Level::DEBUG, "no usage data"));
}

#[tokio::test]
async fn a_non_empty_export_reaches_the_destination_with_its_window_and_filename() {
    let mut recorder = Recorder::default();
    recorder.rows = usage_rows(2);
    let logger = build(&environment(&[]), &recorder, None);
    logger
        .export_window(&window(), None)
        .await
        .expect("the fake export succeeds");
    assert_eq!(recorder.delivered(), vec![b"rows:2".to_vec()]);
    let delivered = recorder
        .calls()
        .into_iter()
        .find_map(|call| match call {
            EngineCall::Deliver {
                start,
                end,
                frequency,
                filename,
                ..
            } => Some((start, end, frequency, filename)),
            _ => None,
        })
        .expect("the export delivers once");
    assert_eq!(
        delivered,
        (
            day("2024-03-09"),
            day("2024-03-10"),
            String::from("daily"),
            String::from("mavvrik_focus_export.csv"),
        )
    );
}
