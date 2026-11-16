#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_roi::records::ROIEstimateStatus;

use super::support::{
    answering_caller, inputs, manager, pull, pull_key, pull_param, record_text, report_of,
    report_text, settled,
};
use crate::roi_cache::store::REPORT_PARAM;

#[tokio::test]
async fn saved_record_is_reused_without_reading_evidence() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12]);
    let key = pull_key(&inputs.settings, "o/r", &pull(12)).await;
    inputs.repository = inputs
        .repository
        .with_param(&pull_param(&key), &record_text(12, Some(key.as_str())));
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let report = report_of(&inputs.repository);
    let record = &report.pulls[0];
    assert_eq!(record.estimate.status, ROIEstimateStatus::Estimated);
    assert_eq!(record.estimate.cached, Some(true));
    assert_eq!(record.estimate.hours.unwrap().to_bits(), 2.0_f64.to_bits());
    assert_eq!(record.cache_key.as_deref(), Some(key.as_str()));
    assert!(!record.incomplete_metadata);
    let requested = inputs.routes.requested();
    assert!(requested.iter().any(|path| path == "/users/dev"));
    assert!(
        !requested
            .iter()
            .any(|path| path == "/repos/o/r/pulls/12/files")
    );
    assert!(inputs.repository.written_named(&pull_param(&key)).is_some());
    let status = manager.status();
    assert_eq!(status.reused, 1);
    assert_eq!(status.estimated, 1);
    assert_eq!(status.needs_attention, 0);
}

#[tokio::test]
async fn saved_record_wins_over_the_previous_report() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12]);
    let key = pull_key(&inputs.settings, "o/r", &pull(12)).await;
    let previous = report_text(&[(12, key.as_str())]).replace("\"hours\":2.0", "\"hours\":9.0");
    inputs.repository = inputs
        .repository
        .with_param(REPORT_PARAM, &previous)
        .with_param(&pull_param(&key), &record_text(12, Some(key.as_str())));
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let report = report_of(&inputs.repository);
    let hours = report.pulls[0].estimate.hours.unwrap();
    assert_eq!(hours.to_bits(), 2.0_f64.to_bits());
}

#[tokio::test]
async fn previous_report_record_is_reused_and_cached() {
    let counted = Arc::new(AtomicUsize::new(0));
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12]);
    inputs.complete = answering_caller(Some(Arc::clone(&counted)));
    let key = pull_key(&inputs.settings, "o/r", &pull(12)).await;
    inputs.repository = inputs
        .repository
        .with_param(REPORT_PARAM, &report_text(&[(12, key.as_str())]));
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let report = report_of(&inputs.repository);
    assert_eq!(report.pulls[0].estimate.cached, Some(true));
    assert_eq!(counted.load(Ordering::SeqCst), 0);
    assert!(inputs.repository.written_named(&pull_param(&key)).is_some());
    assert_eq!(manager.status().reused, 1);
}

#[tokio::test]
async fn cached_record_without_an_estimate_is_reestimated() {
    let counted = Arc::new(AtomicUsize::new(0));
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12]);
    inputs.complete = answering_caller(Some(Arc::clone(&counted)));
    let key = pull_key(&inputs.settings, "o/r", &pull(12)).await;
    let stale = record_text(12, Some(key.as_str()))
        .replace("\"status\":\"estimated\"", "\"status\":\"needs_review\"");
    inputs.repository = inputs.repository.with_param(&pull_param(&key), &stale);
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let report = report_of(&inputs.repository);
    let record = &report.pulls[0];
    assert_eq!(record.estimate.status, ROIEstimateStatus::Estimated);
    assert_eq!(record.estimate.cached, Some(false));
    assert_eq!(record.estimate.hours.unwrap().to_bits(), 3.5_f64.to_bits());
    assert_eq!(counted.load(Ordering::SeqCst), 1);
    assert!(
        inputs
            .routes
            .requested()
            .iter()
            .any(|path| path == "/users/dev")
    );
}

#[tokio::test]
async fn cached_record_without_commit_emails_is_reestimated() {
    let counted = Arc::new(AtomicUsize::new(0));
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12]);
    inputs.complete = answering_caller(Some(Arc::clone(&counted)));
    let key = pull_key(&inputs.settings, "o/r", &pull(12)).await;
    let stale = record_text(12, Some(key.as_str())).replace(
        "\"commit_emails\":[\"work@example.com\"]",
        "\"commit_emails\":null",
    );
    inputs.repository = inputs.repository.with_param(&pull_param(&key), &stale);
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let report = report_of(&inputs.repository);
    assert_eq!(report.pulls[0].estimate.cached, Some(false));
    assert_eq!(counted.load(Ordering::SeqCst), 1);
    assert_eq!(manager.status().reused, 0);
}
