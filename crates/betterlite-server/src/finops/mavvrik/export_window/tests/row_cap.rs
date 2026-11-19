#![cfg(test)]

use betterlite_logging::focus_records::destinations::FocusTimeWindow;
use rstest::rstest;
use tracing::Level;

use crate::finops::mavvrik::support::{
    Recorder, build, capture_logs, day, environment, usage_rows,
};

fn window() -> FocusTimeWindow {
    FocusTimeWindow {
        start_time: day("2024-03-09"),
        end_time: day("2024-03-10"),
        frequency: String::from("daily"),
    }
}

#[tokio::test]
async fn reaching_the_configured_cap_warns_and_names_the_cap_and_the_window() {
    let (logs, _guard) = capture_logs();
    let mut recorder = Recorder::new();
    recorder.rows = usage_rows(3);
    let logger = build(
        &environment(&[("MAVVRIK_FOCUS_MAX_ROWS", "3")]),
        &recorder,
        None,
    );
    logger
        .export_window(&window(), None)
        .await
        .expect("the fake export succeeds");
    assert_eq!(recorder.limits(), vec![3]);
    assert!(logs.has_at(Level::WARN, "row cap reached (3 rows)"));
    assert!(logs.has_at(Level::WARN, "2024-03-09\u{2192}2024-03-10"));
    assert!(logs.has_at(Level::WARN, "Increase MAVVRIK_FOCUS_MAX_ROWS"));
}

#[tokio::test]
async fn one_row_below_the_configured_cap_is_not_a_warning() {
    let (logs, _guard) = capture_logs();
    let mut recorder = Recorder::new();
    recorder.rows = usage_rows(2);
    let logger = build(
        &environment(&[("MAVVRIK_FOCUS_MAX_ROWS", "3")]),
        &recorder,
        None,
    );
    logger
        .export_window(&window(), None)
        .await
        .expect("the fake export succeeds");
    assert_eq!(recorder.limits(), vec![3]);
    assert!(!logs.has_at(Level::WARN, "row cap"));
}

#[rstest]
#[case::above(10, 1, false)]
#[case::exact(1, 1, true)]
#[case::below(2, 1, false)]
#[tokio::test]
async fn an_explicit_limit_replaces_the_configured_cap(
    #[case] limit: i64,
    #[case] rows: usize,
    #[case] warns: bool,
) {
    let (logs, _guard) = capture_logs();
    let mut recorder = Recorder::new();
    recorder.rows = usage_rows(rows);
    let logger = build(
        &environment(&[("MAVVRIK_FOCUS_MAX_ROWS", "3")]),
        &recorder,
        None,
    );
    logger
        .export_window(&window(), Some(limit))
        .await
        .expect("the fake export succeeds");
    assert_eq!(recorder.limits(), vec![limit]);
    assert_eq!(logs.has_at(Level::WARN, "row cap reached"), warns);
}

#[tokio::test]
async fn a_negative_cap_warns_even_with_no_rows() {
    let (logs, _guard) = capture_logs();
    let recorder = Recorder::default();
    let logger = build(
        &environment(&[("MAVVRIK_FOCUS_MAX_ROWS", "-5")]),
        &recorder,
        None,
    );
    logger
        .export_window(&window(), None)
        .await
        .expect("the fake export succeeds");
    assert_eq!(recorder.limits(), vec![-5]);
    assert!(logs.has_at(Level::WARN, "row cap reached (-5 rows)"));
}
