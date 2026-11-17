#![cfg(test)]

use betterlite_roi::records::ROISyncPhase;

use super::support::{TestClock, drained, inputs, manager, report_of, settled, until};
use crate::roi_cache::key::settings_fingerprint;
use crate::roi_cache::store::REPORT_PARAM;

const PULL_FILES: &str = "/repos/o/r/pulls/12/files";

#[tokio::test]
async fn run_publishes_the_report_once_estimates_finish() {
    let clock = TestClock::at("2026-01-10T00:00:00Z");
    let manager = manager(Some(clock.handle()));
    let mut inputs = inputs("o/r", &[12, 13]);
    inputs.settings.backfill_days = 7;
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let report = report_of(&inputs.repository);
    assert_eq!(report.mode, "live");
    assert_eq!(report.start, "2026-01-04");
    assert_eq!(report.end, "2026-01-10");
    assert_eq!(report.synced_at, "2026-01-10T00:00:00+00:00");
    assert_eq!(report.repos, vec!["o/r".to_owned()]);
    assert_eq!(report.estimator_model, "gpt-5.2");
    assert_eq!(report.estimator_prompt, "Estimate this.");
    assert_eq!(report.effort_basis, "without_ai");
    assert_eq!(report.spend.len(), 1);
    assert_eq!(
        report.settings_fingerprint,
        settings_fingerprint(&inputs.settings).unwrap()
    );
    assert_eq!(report.warnings, Some(Vec::new()));
    assert_eq!(report.unavailable_repos, Some(Vec::new()));
    assert!(report.id.is_none());
    let status = manager.status();
    assert_eq!(status.phase, ROISyncPhase::Complete);
    assert_eq!(status.stage, "Analysis complete");
    assert!(!status.running);
    assert!(status.finished_at.is_some());
    assert_eq!(status.estimated, 2);
}

#[tokio::test]
async fn run_stamps_synced_at_after_the_estimates() {
    let clock = TestClock::at("2026-01-10T00:00:00Z");
    let manager = manager(Some(clock.handle()));
    let inputs = inputs("o/r", &[12]);
    let gate = inputs.routes.gate(PULL_FILES);
    assert!(manager.start(inputs.run()).await);
    until(|| {
        inputs
            .routes
            .requested()
            .iter()
            .any(|path| path == PULL_FILES)
    })
    .await;
    clock.set("2026-01-10T02:00:00Z");
    gate.notify_one();
    settled(&manager).await;
    let report = report_of(&inputs.repository);
    assert_eq!(report.synced_at, "2026-01-10T02:00:00+00:00");
    assert_eq!(report.start, "2026-01-04");
    assert_eq!(report.end, "2026-01-10");
    assert_eq!(
        manager.status().started_at.as_deref(),
        Some("2026-01-10T00:00:00+00:00")
    );
}

#[tokio::test]
async fn run_publishes_only_after_every_worker_finished() {
    let manager = manager(None);
    let inputs = inputs("o/r", &[12, 13, 14]);
    let gate = inputs.routes.gate(PULL_FILES);
    assert!(manager.start(inputs.run()).await);
    until(|| {
        inputs
            .routes
            .requested()
            .iter()
            .any(|path| path == PULL_FILES)
    })
    .await;
    drained().await;
    assert!(manager.status().running);
    assert!(inputs.repository.written_named(REPORT_PARAM).is_none());
    gate.notify_one();
    settled(&manager).await;
    assert_eq!(report_of(&inputs.repository).pulls.len(), 3);
}
