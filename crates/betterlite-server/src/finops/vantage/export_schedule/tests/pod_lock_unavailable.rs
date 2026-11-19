#![cfg(test)]

use crate::finops::focus::export_schedule::FocusPodLock;
use crate::finops::vantage::export_schedule::initialize_vantage_export_job;
use crate::finops::vantage::support::{self, Recorder};

#[tokio::test]
async fn without_a_pod_lock_the_vantage_export_runs_unlocked() {
    let recorder = Recorder::new(false);
    let logger = support::build(
        &support::environment(&[]),
        support::params(),
        &recorder,
        None::<std::sync::Arc<dyn FocusPodLock>>,
    );
    initialize_vantage_export_job(&logger)
        .await
        .expect("the fake export succeeds");
    assert_eq!(recorder.calls().len(), 1);
}

#[tokio::test]
async fn without_a_redis_cache_the_vantage_export_runs_unlocked() {
    let recorder = Recorder::new(false);
    let (logger, lock) = support::locked(&recorder, false, Some(true));
    initialize_vantage_export_job(&logger)
        .await
        .expect("the fake export succeeds");
    assert_eq!(recorder.calls().len(), 1);
    assert!(lock.calls().is_empty());
}
