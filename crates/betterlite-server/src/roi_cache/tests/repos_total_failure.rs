#![cfg(test)]

use rstest::rstest;

use crate::roi_cache::error::ReportBlocked;
use crate::roi_cache::repositories::read_repositories;

use super::support::{Routes, day, github, pulls_body};

const TOTAL_FAILURE: &str = "GitHub could not read any selected repository. No new report was published; check repository access or try analysis again later.";

#[rstest]
#[case::one_repository(&["a/b"])]
#[case::two_repositories(&["a/b", "c/d"])]
#[tokio::test]
async fn every_unreadable_repository_blocks_the_report(#[case] repos: &[&str]) {
    let github = github(Routes::default());
    let repos: Vec<String> = repos.iter().map(|repo| (*repo).to_owned()).collect();
    let error = read_repositories(&github, &repos, day("2026-01-01"), day("2026-01-31"))
        .await
        .err()
        .unwrap();
    assert!(matches!(error, ReportBlocked::NoRepositoryReadable));
    assert_eq!(error.to_string(), TOTAL_FAILURE);
}

#[tokio::test]
async fn no_selected_repository_blocks_the_report() {
    let github = github(Routes::default());
    let error = read_repositories(&github, &[], day("2026-01-01"), day("2026-01-31"))
        .await
        .err()
        .unwrap();
    assert!(matches!(error, ReportBlocked::NoRepositoryReadable));
}

#[tokio::test]
async fn one_readable_repository_keeps_the_report_possible() {
    let routes = Routes::default().with("/repos/a/b/pulls", 200, &pulls_body(&[1]));
    let github = github(routes);
    let repos = vec!["a/b".to_owned(), "c/d".to_owned()];
    let batch = read_repositories(&github, &repos, day("2026-01-01"), day("2026-01-31"))
        .await
        .unwrap();
    assert_eq!(batch.queue.len(), 1);
}
