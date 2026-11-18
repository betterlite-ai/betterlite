#![cfg(test)]

use tracing::Level;

use crate::finops::focus::support::{self, Recorder};

#[test]
fn an_unreadable_interval_warns_and_falls_back_to_sixty_seconds() {
    let (logs, _guard) = support::capture_logs();
    let recorder = Recorder::new(false);
    let logger = support::build(
        &support::environment(&[("FOCUS_INTERVAL_SECONDS", "half")]),
        support::params(),
        recorder.factory(),
        None,
    );
    assert_eq!(logger.interval_seconds, 60);
    assert!(logs.has_at(
        Level::WARN,
        "Invalid FOCUS_INTERVAL_SECONDS value: half, ignoring"
    ));
}

#[test]
fn a_readable_interval_is_used_without_a_warning() {
    let (logs, _guard) = support::capture_logs();
    let recorder = Recorder::new(false);
    let logger = support::build(
        &support::environment(&[("FOCUS_INTERVAL_SECONDS", "120")]),
        support::params(),
        recorder.factory(),
        None,
    );
    assert_eq!(logger.interval_seconds, 120);
    assert!(!logs.has_at(Level::WARN, "Invalid FOCUS_INTERVAL_SECONDS"));
}

#[test]
fn an_empty_interval_falls_back_to_sixty_seconds_without_a_warning() {
    let (logs, _guard) = support::capture_logs();
    let recorder = Recorder::new(false);
    let logger = support::build(
        &support::environment(&[("FOCUS_INTERVAL_SECONDS", "")]),
        support::params(),
        recorder.factory(),
        None,
    );
    assert_eq!(logger.interval_seconds, 60);
    assert!(!logs.has_at(Level::WARN, "Invalid FOCUS_INTERVAL_SECONDS"));
}
