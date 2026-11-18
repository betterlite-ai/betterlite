#![cfg(test)]

use crate::roi_settings::error::RoiSettingsError;
use crate::roi_settings::store::REPORT_KEY;
use crate::roi_settings::store::load_report;
use crate::roi_settings::support::MemoryRepository;

const REPORT: &str = r#"{"mode":"backfill","start":"2026-01-01","end":"2026-01-31","synced_at":"2026-02-01T00:00:00Z","repos":["owner/repo"],"estimator_model":"gpt-4o","estimator_prompt":"Estimate.","effort_basis":"estimated_hours","spend":[],"pulls":[],"settings_fingerprint":"fingerprint"}"#;

#[tokio::test]
async fn load_report_without_a_parameter_is_none() {
    assert!(
        load_report(&MemoryRepository::default())
            .await
            .expect("no report")
            .is_none()
    );
}

#[tokio::test]
async fn load_report_reads_the_stored_report() {
    let repository = MemoryRepository::default();
    repository.seed(REPORT_KEY, REPORT);
    let report = load_report(&repository)
        .await
        .expect("loaded report")
        .expect("a report");
    assert_eq!(report.mode, "backfill");
    assert_eq!(report.repos, ["owner/repo"]);
    assert_eq!(report.settings_fingerprint, "fingerprint");
}

#[tokio::test]
async fn load_report_rejects_an_invalid_report() {
    let repository = MemoryRepository::default();
    repository.seed(REPORT_KEY, r#"{"mode":"backfill"}"#);
    assert!(matches!(
        load_report(&repository).await,
        Err(RoiSettingsError::StoredReport)
    ));
}
