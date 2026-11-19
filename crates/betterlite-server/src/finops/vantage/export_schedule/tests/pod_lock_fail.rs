#![cfg(test)]

use tracing::Level;

use crate::finops::vantage::export_schedule::initialize_vantage_export_job;
use crate::finops::vantage::support::{self, PodLockCall, Recorder};

#[tokio::test]
async fn a_lock_held_elsewhere_skips_the_vantage_export() {
    let (logs, _guard) = support::capture_logs();
    let recorder = Recorder::new(false);
    let (logger, lock) = support::locked(&recorder, true, Some(false));
    initialize_vantage_export_job(&logger)
        .await
        .expect("a skipped run is not an error");
    assert!(recorder.calls().is_empty());
    assert_eq!(
        lock.calls(),
        vec![PodLockCall::Acquire("vantage_export_usage_data".to_owned())]
    );
    assert!(logs.has_at(Level::DEBUG, "Vantage export: unable to acquire pod lock"));
}

#[tokio::test]
async fn a_lock_that_never_answers_skips_the_vantage_export() {
    let recorder = Recorder::new(false);
    let (logger, lock) = support::locked(&recorder, true, None);
    initialize_vantage_export_job(&logger)
        .await
        .expect("a skipped run is not an error");
    assert!(recorder.calls().is_empty());
    assert_eq!(
        lock.calls(),
        vec![PodLockCall::Acquire("vantage_export_usage_data".to_owned())]
    );
}
