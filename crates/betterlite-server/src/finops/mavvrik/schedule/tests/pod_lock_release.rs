#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Notify;
use tokio::time::timeout;

use crate::finops::mavvrik::schedule::initialize_mavvrik_export_job;
use crate::finops::mavvrik::support::{PodLockCall, Recorder, build, environment, pod_lock};

#[tokio::test]
async fn the_lock_is_released_when_the_export_fails() {
    let mut recorder = Recorder::new();
    recorder.fail = true;
    let (lock, port) = pod_lock(true, Some(true));
    let logger = build(&environment(&[]), &recorder, Some(port));
    initialize_mavvrik_export_job(&logger)
        .await
        .expect_err("the export fails");
    assert_eq!(
        lock.calls(),
        vec![
            PodLockCall::Acquire(String::from("mavvrik_focus_export_usage_data")),
            PodLockCall::Release(String::from("mavvrik_focus_export_usage_data")),
        ]
    );
}

#[tokio::test]
async fn a_cancelled_export_still_releases_the_lock() {
    let mut recorder = Recorder::new();
    let started = Arc::new(Notify::new());
    recorder.gate = Some(Arc::clone(&started));
    let (lock, port) = pod_lock(true, Some(true));
    let logger = Arc::new(build(&environment(&[]), &recorder, Some(port)));
    let job = logger.tasks.spawn({
        let task_logger = Arc::clone(&logger);
        async move { initialize_mavvrik_export_job(&task_logger).await }
    });
    started.notified().await;
    job.abort();
    let cancelled = job.await.expect_err("the run is cancelled");
    assert!(cancelled.is_cancelled());
    let drained = timeout(Duration::from_secs(1), logger.drain()).await;
    assert!(drained.is_ok(), "the drained tracker finishes");
    assert!(logger.tasks.is_closed());
    assert_eq!(
        lock.calls(),
        vec![
            PodLockCall::Acquire(String::from("mavvrik_focus_export_usage_data")),
            PodLockCall::Release(String::from("mavvrik_focus_export_usage_data")),
        ]
    );
    assert!(recorder.windows().is_empty());
}
