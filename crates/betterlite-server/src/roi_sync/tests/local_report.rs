#![cfg(test)]

use super::support::{inputs, manager, report_of, settings, settled};
use crate::roi_cache::key::settings_fingerprint;

#[tokio::test]
async fn the_stored_report_records_settings_spend_and_pulls() {
    let manager = manager(None);
    let inputs = inputs("o/r", &[12]);
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let report = report_of(&inputs.repository);
    assert_eq!(report.mode, "live");
    assert_eq!(report.effort_basis, "without_ai");
    assert_eq!(report.estimator_model, "gpt-5.2");
    assert_eq!(report.estimator_prompt, "Estimate this.");
    assert_eq!(
        report.settings_fingerprint,
        settings_fingerprint(&inputs.settings).unwrap()
    );
    assert_eq!(report.spend.len(), 1);
    assert_eq!(report.spend[0].email, "dev@example.com");
    assert_eq!(report.spend[0].requests, 2);
    assert_eq!(report.pulls.len(), 1);
    assert_eq!(report.pulls[0].number, 12);
}

#[tokio::test]
async fn a_partial_report_names_the_unavailable_repositories() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12]);
    inputs.settings = settings(&["o/r", "c/d"]);
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let report = report_of(&inputs.repository);
    assert_eq!(report.unavailable_repos, Some(vec!["c/d".to_owned()]));
    let warnings = report.warnings.unwrap();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].starts_with("Incomplete report: could not read c/d."));
    assert_eq!(report.pulls.len(), 1);
}
