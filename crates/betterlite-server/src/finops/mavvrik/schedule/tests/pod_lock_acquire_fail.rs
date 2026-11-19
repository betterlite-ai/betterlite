#![cfg(test)]

use rstest::rstest;
use tracing::Level;

use crate::finops::mavvrik::schedule::initialize_mavvrik_export_job;
use crate::finops::mavvrik::support::{
    PodLockCall, Recorder, build, capture_logs, environment, pod_lock,
};

#[rstest]
#[case::refused(Some(false))]
#[case::unavailable(None)]
#[tokio::test]
async fn a_lock_that_is_not_acquired_logs_and_exports_nothing(#[case] acquired: Option<bool>) {
    let (logs, _guard) = capture_logs();
    let recorder = Recorder::default();
    let (lock, port) = pod_lock(true, acquired);
    let logger = build(&environment(&[]), &recorder, Some(port));
    initialize_mavvrik_export_job(&logger)
        .await
        .expect("a refused lease is not an error");
    assert_eq!(
        lock.calls(),
        vec![PodLockCall::Acquire(String::from(
            "mavvrik_focus_export_usage_data"
        ))]
    );
    assert!(recorder.calls().is_empty());
    assert!(logs.has_at(
        Level::DEBUG,
        "Mavvrik FOCUS export: unable to acquire pod lock"
    ));
}
