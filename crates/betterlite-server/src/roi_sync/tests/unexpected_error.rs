#![cfg(test)]

use betterlite_roi::records::{ROISyncPhase, ROISyncStatus};

use super::support::{FakeSpend, MemoryRepository, inputs, manager, settled};
use crate::roi_cache::store::REPORT_PARAM;
use crate::roi_sync::error::GENERIC_MESSAGE;

fn assert_generic_failure(status: &ROISyncStatus) {
    assert_eq!(status.phase, ROISyncPhase::Error);
    assert_eq!(status.stage, "Sync failed");
    assert_eq!(status.error.as_deref(), Some(GENERIC_MESSAGE));
    assert!(!status.running);
    assert!(status.finished_at.is_some());
}

#[tokio::test]
async fn a_spend_failure_uses_the_generic_message() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12]);
    inputs.spend = FakeSpend::failing();
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    assert_generic_failure(&manager.status());
    assert!(inputs.repository.written_named(REPORT_PARAM).is_none());
}

#[tokio::test]
async fn a_cache_write_failure_uses_the_generic_message() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12]);
    inputs.repository = MemoryRepository::refusing_writes();
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    assert_generic_failure(&manager.status());
    assert!(inputs.repository.written().is_empty());
}

#[tokio::test]
async fn a_repository_read_failure_uses_the_generic_message() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12]);
    inputs.repository = MemoryRepository::refusing_reads();
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    assert_generic_failure(&manager.status());
    assert!(inputs.repository.written_named(REPORT_PARAM).is_none());
}
