#![cfg(test)]

use betterlite_roi::records::{ROIEstimate, ROIEstimateStatus, ROIPullRecord};

use crate::roi_cache::fallback::unavailable_record;
use crate::roi_github::error::SourceError;

use super::support::{Routes, github, pull_item};

#[tokio::test]
async fn the_record_asks_for_a_retry_and_carries_the_pull_and_profile() {
    let routes = Routes::default().with("/users/dev", 200, r#"{"email":"Dev@Example.com"}"#);
    let github = github(routes);
    let pull = pull_item(7, Some("dev"), Some("abc123"));
    let error = SourceError::Http {
        label: "GitHub returned an error.",
        status: 500,
    };
    let record = unavailable_record(&github, "o/r", &pull, &error).await;
    let expected = ROIPullRecord {
        repo: "o/r".to_owned(),
        number: 7,
        title: "Pull 7".to_owned(),
        url: "https://github.com/o/r/pull/7".to_owned(),
        login: "dev".to_owned(),
        emails: vec!["dev@example.com".to_owned()],
        profile_email: "dev@example.com".to_owned(),
        commit_emails: Some(Vec::new()),
        merged_at: "2026-01-05T00:00:00Z".to_owned(),
        head_sha: "abc123".to_owned(),
        additions: 0,
        deletions: 0,
        changed_files: 0,
        commit_count: 0,
        incomplete_metadata: true,
        estimate: ROIEstimate {
            status: ROIEstimateStatus::NeedsReview,
            hours: None,
            reasoning: "PR metadata could not be read: GitHub returned an error. (HTTP 500) Run analysis again to retry this PR.".to_owned(),
            model: None,
            evidence_source: None,
            effort_basis: None,
            cached: None,
        },
        cache_key: None,
    };
    assert_eq!(record, expected);
}

#[tokio::test]
async fn without_a_profile_email_the_record_has_no_emails() {
    let github = github(Routes::default());
    let pull = pull_item(7, Some("dev"), Some("abc123"));
    let record = unavailable_record(&github, "o/r", &pull, &SourceError::PaginationLimit).await;
    assert_eq!(record.profile_email, "");
    assert!(record.emails.is_empty());
}

#[tokio::test]
async fn an_unmerged_pull_without_a_head_uses_its_update_time_and_an_empty_sha() {
    let github = github(Routes::default());
    let mut pull = pull_item(7, Some("dev"), None);
    pull.merged_at = None;
    let record = unavailable_record(&github, "o/r", &pull, &SourceError::PaginationLimit).await;
    assert_eq!(record.merged_at, "2026-01-06T00:00:00Z");
    assert_eq!(record.head_sha, "");
}
