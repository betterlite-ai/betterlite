#![cfg(test)]

use betterlite_roi::records::ROISyncPhase;
use chrono::NaiveDate;

use super::support::{FakeSpend, TestClock, inputs, manager, settled};
use crate::roi_sync::error::GENERIC_MESSAGE;

#[tokio::test]
async fn run_reads_spend_for_the_configured_backfill_window() {
    let clock = TestClock::at("2026-01-10T12:00:00Z");
    let manager = manager(Some(clock.handle()));
    let mut inputs = inputs("o/r", &[12]);
    inputs.settings.backfill_days = 3;
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let ranges = inputs.spend.ranges();
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0].0, NaiveDate::from_ymd_opt(2026, 1, 8).unwrap());
    assert_eq!(ranges[0].1, NaiveDate::from_ymd_opt(2026, 1, 10).unwrap());
    assert_eq!(manager.status().phase, ROISyncPhase::Complete);
}

#[tokio::test]
async fn run_reads_spend_before_any_repository() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12]);
    inputs.spend = FakeSpend::failing();
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    assert!(inputs.routes.requested().is_empty());
    let status = manager.status();
    assert_eq!(status.phase, ROISyncPhase::Error);
    assert_eq!(status.stage, "Sync failed");
    assert_eq!(status.error.as_deref(), Some(GENERIC_MESSAGE));
}
