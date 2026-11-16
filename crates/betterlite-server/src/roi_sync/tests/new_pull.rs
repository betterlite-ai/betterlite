#![cfg(test)]

use betterlite_roi::records::ROIEstimateStatus;

use super::support::{inputs, manager, pull, pull_key, pull_param, report_of, settled};
use crate::roi_cache::store::PULL_PARAM_PREFIX;

const DETAIL: &str = "/repos/o/r/pulls/12";

#[tokio::test]
async fn new_pull_estimates_from_evidence_and_caches() {
    let manager = manager(None);
    let inputs = inputs("o/r", &[12]);
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let key = pull_key(&inputs.settings, "o/r", &pull(12)).await;
    let report = report_of(&inputs.repository);
    let record = &report.pulls[0];
    assert_eq!(record.repo, "o/r");
    assert_eq!(record.number, 12);
    assert_eq!(record.title, "Pull 12");
    assert_eq!(record.url, "https://github.com/o/r/pull/12");
    assert_eq!(record.login, "dev");
    assert_eq!(record.merged_at, "2026-01-05T00:00:00Z");
    assert_eq!(record.head_sha, "sha12");
    assert_eq!(record.additions, 10);
    assert_eq!(record.deletions, 2);
    assert_eq!(record.changed_files, 1);
    assert_eq!(record.commit_count, 1);
    assert!(!record.incomplete_metadata);
    assert_eq!(
        record.emails,
        vec!["dev@example.com".to_owned(), "work@example.com".to_owned()]
    );
    assert_eq!(record.profile_email, "dev@example.com");
    assert_eq!(
        record.commit_emails,
        Some(vec!["work@example.com".to_owned()])
    );
    assert_eq!(record.estimate.status, ROIEstimateStatus::Estimated);
    assert_eq!(record.estimate.cached, Some(false));
    assert_eq!(record.estimate.model.as_deref(), Some("gpt-5.2"));
    assert_eq!(
        record.estimate.evidence_source.as_deref(),
        Some("pr_metadata")
    );
    assert_eq!(record.estimate.effort_basis.as_deref(), Some("without_ai"));
    assert_eq!(record.cache_key.as_deref(), Some(key.as_str()));
    assert!(inputs.repository.written_named(&pull_param(&key)).is_some());
}

#[tokio::test]
async fn new_pull_reads_the_detail_files_and_commits() {
    let manager = manager(None);
    let inputs = inputs("o/r", &[12]);
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let requested = inputs.routes.requested();
    assert!(requested.iter().any(|path| path == DETAIL));
    assert!(
        requested
            .iter()
            .any(|path| path == "/repos/o/r/pulls/12/files")
    );
    assert!(
        requested
            .iter()
            .any(|path| path == "/repos/o/r/pulls/12/commits")
    );
}

#[tokio::test]
async fn new_pull_without_a_head_is_not_cached() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12]);
    let headless = r#"{"number":12,"title":"Pull 12","body":"Body","html_url":"https://github.com/o/r/pull/12","user":{"login":"dev"},"merged_at":"2026-01-05T00:00:00Z","head":{},"additions":10,"deletions":2,"changed_files":1,"commits":1}"#;
    inputs.routes = inputs.routes.clone().with(DETAIL, 200, headless);
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let report = report_of(&inputs.repository);
    assert!(report.pulls[0].cache_key.is_none());
    let cached = inputs
        .repository
        .written()
        .iter()
        .filter(|entry| entry.0.starts_with(PULL_PARAM_PREFIX))
        .count();
    assert_eq!(cached, 0);
}
