#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_roi::records::ROISyncPhase;

use super::support::{
    MemoryRepository, answering_caller, failing_caller, inputs, manager, pull, pull_key,
    pull_param, record_text, report_of, settled,
};
use crate::roi_cache::error::ReportBlocked;
use crate::roi_cache::store::REPORT_PARAM;

#[tokio::test]
async fn run_estimates_every_pull_and_counts_progress() {
    let counted = Arc::new(AtomicUsize::new(0));
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12, 13]);
    inputs.complete = answering_caller(Some(Arc::clone(&counted)));
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let status = manager.status();
    assert_eq!(status.phase, ROISyncPhase::Complete);
    assert_eq!(status.total, 2);
    assert_eq!(status.done, 2);
    assert_eq!(status.estimated, 2);
    assert_eq!(status.reused, 0);
    assert_eq!(status.needs_attention, 0);
    assert_eq!(counted.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn run_counts_reused_pulls_separately() {
    let counted = Arc::new(AtomicUsize::new(0));
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12, 13]);
    inputs.complete = answering_caller(Some(Arc::clone(&counted)));
    let first = pull_key(&inputs.settings, "o/r", &pull(12)).await;
    let second = pull_key(&inputs.settings, "o/r", &pull(13)).await;
    inputs.repository = inputs
        .repository
        .with_param(&pull_param(&first), &record_text(12, Some(first.as_str())))
        .with_param(
            &pull_param(&second),
            &record_text(13, Some(second.as_str())),
        );
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let status = manager.status();
    assert_eq!(status.phase, ROISyncPhase::Complete);
    assert_eq!(status.done, 2);
    assert_eq!(status.estimated, 2);
    assert_eq!(status.reused, 2);
    assert_eq!(status.needs_attention, 0);
    assert_eq!(counted.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn run_fails_when_no_estimate_succeeds() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12]);
    inputs.complete = failing_caller();
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let expected = ReportBlocked::NoEstimates.to_string();
    let status = manager.status();
    assert_eq!(status.phase, ROISyncPhase::Error);
    assert_eq!(status.done, 1);
    assert_eq!(status.estimated, 0);
    assert_eq!(status.needs_attention, 1);
    assert_eq!(status.error.as_deref(), Some(expected.as_str()));
    assert!(inputs.repository.written_named(REPORT_PARAM).is_none());
}

#[tokio::test]
async fn run_completes_with_attention_counts_when_one_estimate_fails() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12, 13]);
    inputs.complete = failing_caller();
    let key = pull_key(&inputs.settings, "o/r", &pull(12)).await;
    inputs.repository =
        MemoryRepository::holding_text(&pull_param(&key), &record_text(12, Some(key.as_str())));
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let status = manager.status();
    assert_eq!(status.phase, ROISyncPhase::Complete);
    assert_eq!(status.done, 2);
    assert_eq!(status.estimated, 1);
    assert_eq!(status.reused, 1);
    assert_eq!(status.needs_attention, 1);
    assert_eq!(report_of(&inputs.repository).pulls.len(), 2);
}
