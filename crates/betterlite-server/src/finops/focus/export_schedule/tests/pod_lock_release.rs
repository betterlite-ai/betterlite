#![cfg(test)]

use crate::finops::focus::error::FocusError;
use crate::finops::focus::support::{self, PodLockCall, Recorder};

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
