#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Notify;
use tokio::time::timeout;
use tokio_util::task::TaskTracker;

use crate::finops::focus::export_schedule::{FocusPodLock, ScheduleError};
use crate::finops::vantage::error::VantageExportError;
use crate::finops::vantage::export_schedule::initialize_vantage_export_job;
use crate::finops::vantage::support::{self, PodLockCall, Recorder};

#[tokio::test]
async fn a_failed_export_still_releases_the_vantage_lock() {
    let recorder = Recorder::new(true);
    let (logger, lock) = support::locked(&recorder, true, Some(true));
    let error = initialize_vantage_export_job(&logger)
        .await
        .expect_err("the fake export fails");
    assert!(matches!(&error, VantageExportError::Export(_)));
    assert_eq!(error.to_string(), "the VANTAGE export failed");
    assert_eq!(recorder.calls().len(), 1);
    assert_eq!(
        lock.calls(),
        vec![
            PodLockCall::Acquire("vantage_export_usage_data".to_owned()),
            PodLockCall::Release("vantage_export_usage_data".to_owned()),
        ]
    );
}

#[tokio::test]
async fn an_unsupported_frequency_still_releases_the_vantage_lock() {
    let recorder = Recorder::new(false);
    let lock = Arc::new(support::FakePodLock::new(true, Some(true)));
    let pod_lock: Arc<dyn FocusPodLock> = Arc::clone(&lock);
    let mut params = support::params();
    params.frequency = Some("weekly".to_owned());
    let logger = support::build(
        &support::environment(&[]),
        params,
        &recorder,
        Some(pod_lock),
    );
    let error = initialize_vantage_export_job(&logger)
        .await
        .expect_err("weekly has no time window");
    assert!(matches!(
        &error,
        VantageExportError::Schedule(ScheduleError::UnsupportedFrequency { frequency })
            if frequency == "weekly"
    ));
    assert!(recorder.calls().is_empty());
    assert_eq!(
        lock.calls(),
        vec![
            PodLockCall::Acquire("vantage_export_usage_data".to_owned()),
            PodLockCall::Release("vantage_export_usage_data".to_owned()),
        ]
    );
}

#[tokio::test]
async fn draining_waits_for_the_release_task() {
    let recorder = Recorder::new(false);
    let logger = Arc::new(support::build(
        &support::environment(&[]),
        support::params(),
        &recorder,
        None,
    ));
    let release = Arc::new(Notify::new());
    logger.tasks.spawn({
        let release = Arc::clone(&release);
        async move { release.notified().await }
    });
    let waiting = TaskTracker::new();
    let drain = waiting.spawn({
        let logger = Arc::clone(&logger);
        async move { logger.drain().await }
    });
    tokio::task::yield_now().await;
    assert!(!drain.is_finished(), "the drain waits for the release task");
    release.notify_one();
    timeout(Duration::from_secs(10), drain)
        .await
        .expect("the drain finishes")
        .expect("the drain task is not cancelled");
}

#[tokio::test]
async fn a_cancelled_export_still_releases_the_vantage_lock() {
    let started = Arc::new(Notify::new());
    let recorder = Recorder::paused(Arc::clone(&started));
    let lock = Arc::new(support::FakePodLock::new(true, Some(true)));
    let pod_lock: Arc<dyn FocusPodLock> = Arc::clone(&lock);
    let logger = Arc::new(support::build(
        &support::environment(&[]),
        support::params(),
        &recorder,
        Some(pod_lock),
    ));
    let tasks = TaskTracker::new();
    let job = tasks.spawn({
        let logger = Arc::clone(&logger);
        async move { initialize_vantage_export_job(&logger).await }
    });
    started.notified().await;
    job.abort();
    job.await.expect_err("the export is cancelled");
    timeout(Duration::from_secs(10), logger.drain())
        .await
        .expect("the release finishes while draining");
    assert_eq!(recorder.calls().len(), 1);
    assert_eq!(
        lock.calls(),
        vec![
            PodLockCall::Acquire("vantage_export_usage_data".to_owned()),
            PodLockCall::Release("vantage_export_usage_data".to_owned()),
        ]
    );
}
