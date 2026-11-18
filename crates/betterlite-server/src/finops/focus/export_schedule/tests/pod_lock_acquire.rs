#![cfg(test)]

use crate::finops::focus::support::{self, ExportCall, PodLockCall, Recorder};

#[tokio::test]
async fn an_acquired_lock_runs_the_export_and_releases_it() {
    let (logger, recorder, lock) = support::locked(Recorder::new(false), true, Some(true));
    logger
        .initialize_focus_export_job()
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
