#![cfg(test)]

use std::collections::BTreeMap;

use crate::finops::focus::support::{self, Recorder};

#[tokio::test]
async fn without_a_pod_lock_the_export_runs_unlocked() {
    let recorder = Recorder::new(false);
    let logger = support::build(
        &BTreeMap::new(),
        support::params(),
        recorder.factory(),
        None,
    );
    logger
        .initialize_focus_export_job()
        .await
        .expect("the fake export succeeds");
    assert_eq!(recorder.calls().len(), 1);
}

#[tokio::test]
async fn without_a_redis_cache_the_export_runs_unlocked() {
    let (logger, recorder, lock) = support::locked(Recorder::new(false), false, Some(true));
    logger
        .initialize_focus_export_job()
        .await
        .expect("the fake export succeeds");
    assert_eq!(recorder.calls().len(), 1);
    assert!(lock.calls().is_empty());
}
