#![cfg(test)]

use crate::finops::vantage::export_schedule::initialize_vantage_export_job;
use crate::finops::vantage::support::{self, ExportCall, PodLockCall, Recorder, at};

#[tokio::test]
async fn the_vantage_lease_uses_the_vantage_job_name() {
    let recorder = Recorder::new(false);
    let (logger, lock) = support::locked(&recorder, true, Some(true));
    initialize_vantage_export_job(&logger)
        .await
        .expect("the fake export succeeds");
    assert_eq!(
        lock.calls(),
        vec![
            PodLockCall::Acquire("vantage_export_usage_data".to_owned()),
            PodLockCall::Release("vantage_export_usage_data".to_owned()),
        ]
    );
}

#[tokio::test]
async fn the_scheduled_export_covers_the_closed_hour() {
    let recorder = Recorder::new(false);
    let (logger, lock) = support::locked(&recorder, true, Some(true));
    initialize_vantage_export_job(&logger)
        .await
        .expect("the fake export succeeds");
    assert_eq!(
        recorder.calls(),
        vec![ExportCall::Window {
            start: at("2024-03-10T12:00:00Z"),
            end: at("2024-03-10T13:00:00Z"),
            frequency: "hourly".to_owned(),
            limit: None,
        }]
    );
    assert_eq!(lock.calls().len(), 2);
}
