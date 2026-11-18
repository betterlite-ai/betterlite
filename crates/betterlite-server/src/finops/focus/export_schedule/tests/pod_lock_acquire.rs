#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use tokio::sync::Notify;
use tokio_util::task::TaskTracker;

use crate::finops::focus::export_schedule::{
    FOCUS_USAGE_DATA_JOB_NAME, FocusPodLock, initialize_focus_export_job,
};
use crate::finops::focus::support::{self, ExportCall, PodLockCall, Recorder};

#[tokio::test]
async fn an_acquired_lock_runs_the_export_and_releases_it() {
    let (logger, recorder, lock) = support::locked(Recorder::new(false), true, Some(true));
    initialize_focus_export_job(&logger)
        .await
        .expect("the fake export succeeds");
    assert_eq!(
        lock.calls(),
        vec![
            PodLockCall::Acquire("focus_export_usage_data".to_owned()),
            PodLockCall::Release("focus_export_usage_data".to_owned()),
        ]
    );
    assert_eq!(
        recorder.calls(),
        vec![ExportCall::Window {
            start: support::at("2024-03-10T12:00:00Z"),
            end: support::at("2024-03-10T13:00:00Z"),
            frequency: "hourly".to_owned(),
            limit: None,
        }]
    );
}

#[tokio::test]
async fn another_pod_cannot_export_until_the_holder_finishes_and_releases() {
    let shared = support::SharedLockCache::default();
    let lease_owner = support::lock_manager(Some(Arc::new(shared.clone())));
    let rival_pod = support::lock_manager(Some(Arc::new(shared)));
    let started = Arc::new(Notify::new());
    let recorder = Recorder::paused(false, Arc::clone(&started));
    let pod_lock: Arc<dyn FocusPodLock> = lease_owner;
    let logger = Arc::new(support::build(
        &BTreeMap::new(),
        support::params(),
        recorder.factory(),
        Some(pod_lock),
    ));
    let tasks = TaskTracker::new();
    let job = tasks.spawn({
        let logger = Arc::clone(&logger);
        async move { initialize_focus_export_job(&logger).await }
    });
    started.notified().await;
    assert_eq!(
        rival_pod
            .acquire_lock(FOCUS_USAGE_DATA_JOB_NAME, None, false)
            .await,
        Some(false)
    );
    job.abort();
    let cancelled = job.await.expect_err("the export is cancelled");
    assert!(cancelled.is_cancelled());
    logger.drain().await;
    assert_eq!(
        rival_pod
            .acquire_lock(FOCUS_USAGE_DATA_JOB_NAME, None, false)
            .await,
        Some(true)
    );
    assert_eq!(recorder.calls().len(), 1);
}
