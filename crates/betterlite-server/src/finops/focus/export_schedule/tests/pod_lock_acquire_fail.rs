#![cfg(test)]

use tracing::Level;

use crate::finops::focus::export_schedule::initialize_focus_export_job;
use crate::finops::focus::support::{self, PodLockCall, Recorder};

#[tokio::test]
async fn a_lock_held_elsewhere_skips_the_export() {
    let (logs, _guard) = support::capture_logs();
    let (logger, recorder, lock) = support::locked(Recorder::new(false), true, Some(false));
    initialize_focus_export_job(&logger)
        .await
        .expect("a skipped run is not an error");
    assert!(recorder.settings().is_empty());
    assert!(recorder.calls().is_empty());
    assert_eq!(
        lock.calls(),
        vec![PodLockCall::Acquire("focus_export_usage_data".to_owned())]
    );
    assert!(logs.has_at(Level::DEBUG, "Focus export: unable to acquire pod lock"));
}

#[tokio::test]
async fn a_lock_that_never_answers_skips_the_export() {
    let (logs, _guard) = support::capture_logs();
    let (logger, recorder, lock) = support::locked(Recorder::new(false), true, None);
    initialize_focus_export_job(&logger)
        .await
        .expect("a skipped run is not an error");
    assert!(recorder.settings().is_empty());
    assert!(recorder.calls().is_empty());
    assert_eq!(
        lock.calls(),
        vec![PodLockCall::Acquire("focus_export_usage_data".to_owned())]
    );
    assert!(logs.has_at(Level::DEBUG, "Focus export: unable to acquire pod lock"));
}
