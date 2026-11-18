#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Notify;
use tokio::time::timeout;
use tokio_util::task::TaskTracker;

use crate::finops::focus::error::FocusError;
use crate::finops::focus::export::{FocusLogger, FocusLoggerDeps};
use crate::finops::focus::support::{self, FixedClock, PodLockCall, Recorder, at};

#[tokio::test]
async fn a_failed_export_still_releases_the_lock_and_returns_the_error() {
    let (logger, recorder, lock) = support::locked(Recorder::new(true), true, Some(true));
    let error = logger
        .initialize_focus_export_job()
        .await
        .expect_err("the fake export fails");
    assert!(matches!(&error, FocusError::Export(_)));
    assert_eq!(error.to_string(), "the FOCUS export failed");
    assert_eq!(recorder.calls().len(), 1);
    assert_eq!(
        lock.calls(),
        vec![
            PodLockCall::Acquire("focus_export_usage_data".to_owned()),
            PodLockCall::Release("focus_export_usage_data".to_owned()),
        ]
    );
}

#[tokio::test]
async fn a_cancelled_export_still_releases_the_lock_exactly_once() {
    let started = Arc::new(Notify::new());
    let recorder = Recorder::paused(false, Arc::clone(&started));
    let lock = Arc::new(support::FakePodLock::new(true, Some(true)));
    let tasks = TaskTracker::new();
    let logger = Arc::new(
        FocusLogger::new(
            &support::environment(&[]),
            support::params(),
            FocusLoggerDeps {
                factory: recorder.factory(),
                clock: Arc::new(FixedClock {
                    now: at(support::FIXED_NOW),
                }),
                pod_lock: Some(support::as_pod_lock(&lock)),
                tasks: tasks.clone(),
            },
        )
        .expect("the logger builds"),
    );
    let job = tokio::spawn({
        let logger = Arc::clone(&logger);
        async move { logger.initialize_focus_export_job().await }
    });
    started.notified().await;
    job.abort();
    job.await.expect_err("the export is cancelled");
    timeout(Duration::from_secs(10), logger.drain())
        .await
        .expect("the release finishes while draining");
    assert!(tasks.is_closed());
    assert_eq!(recorder.calls().len(), 1);
    assert_eq!(
        lock.calls(),
        vec![
            PodLockCall::Acquire("focus_export_usage_data".to_owned()),
            PodLockCall::Release("focus_export_usage_data".to_owned()),
        ]
    );
}
