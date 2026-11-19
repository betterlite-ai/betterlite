#![cfg(test)]

use crate::finops::mavvrik::schedule::initialize_mavvrik_export_job;
use crate::finops::mavvrik::support::{PodLockCall, Recorder, build, environment, pod_lock};

#[tokio::test]
async fn an_acquired_lock_runs_the_scheduled_export_and_releases_it() {
    let recorder = Recorder::default();
    let (lock, port) = pod_lock(true, Some(true));
    let logger = build(&environment(&[]), &recorder, Some(port));
    initialize_mavvrik_export_job(&logger)
        .await
        .expect("the fake export succeeds");
    assert_eq!(
        lock.calls(),
        vec![
            PodLockCall::Acquire(String::from("mavvrik_focus_export_usage_data")),
            PodLockCall::Release(String::from("mavvrik_focus_export_usage_data")),
        ]
    );
    assert_eq!(recorder.windows().len(), 7);
}

#[tokio::test]
async fn a_lock_manager_without_redis_runs_the_export_without_a_lease() {
    let recorder = Recorder::default();
    let (lock, port) = pod_lock(false, Some(true));
    let logger = build(&environment(&[]), &recorder, Some(port));
    initialize_mavvrik_export_job(&logger)
        .await
        .expect("the fake export succeeds");
    assert!(lock.calls().is_empty());
    assert_eq!(recorder.windows().len(), 7);
}

#[tokio::test]
async fn no_pod_lock_at_all_runs_the_export_directly() {
    let recorder = Recorder::default();
    let logger = build(&environment(&[]), &recorder, None);
    initialize_mavvrik_export_job(&logger)
        .await
        .expect("the fake export succeeds");
    assert_eq!(recorder.windows().len(), 7);
}
