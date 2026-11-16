#![cfg(test)]

use rstest::rstest;

use crate::roi_cache::error::ReportBlocked;
use crate::roi_cache::repositories::read_repositories;

use super::support::{Routes, day, github};

#[rstest]
#[case::one_unreadable(&["a/b", "c/d"], &["/repos/c/d/pulls"], "a/b", "GitHub could not read a/b, and the accessible repositories returned no pull requests. No new report was published; check repository access or try analysis again later.")]
#[case::two_unreadable(&["a/b", "c/d", "e/f"], &["/repos/e/f/pulls"], "a/b, c/d", "GitHub could not read a/b, c/d, and the accessible repositories returned no pull requests. No new report was published; check repository access or try analysis again later.")]
#[tokio::test]
async fn unreadable_repositories_without_any_accessible_pull_block_the_report(
    #[case] names: &[&str],
    #[case] readable_paths: &[&str],
    #[case] unreadable: &str,
    #[case] message: &str,
) {
    let routes = readable_paths
        .iter()
        .fold(Routes::default(), |routes, path| {
            routes.with(path, 200, "[]")
        });
    let github = github(routes);
    let repos: Vec<String> = names.iter().map(|repo| (*repo).to_owned()).collect();
    let error = read_repositories(&github, &repos, day("2026-01-01"), day("2026-01-31"))
        .await
        .err()
        .unwrap();
    assert!(
        matches!(&error, ReportBlocked::NoPullRequests { repos: blocked } if blocked == unreadable)
    );
    assert_eq!(error.to_string(), message);
}

#[tokio::test]
async fn all_readable_repositories_without_pulls_are_an_empty_batch() {
    let routes =
        Routes::default()
            .with("/repos/a/b/pulls", 200, "[]")
            .with("/repos/c/d/pulls", 200, "[]");
    let github = github(routes);
    let repos = vec!["a/b".to_owned(), "c/d".to_owned()];
    let batch = read_repositories(&github, &repos, day("2026-01-01"), day("2026-01-31"))
        .await
        .unwrap();
    assert!(batch.queue.is_empty());
    assert!(batch.unavailable_repos.is_empty());
    assert!(batch.warnings.is_empty());
    assert_eq!(batch.stage, "Analysis complete");
}
