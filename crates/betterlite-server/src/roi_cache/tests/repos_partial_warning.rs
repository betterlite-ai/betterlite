#![cfg(test)]

use crate::roi_cache::repositories::read_repositories;

use super::support::{Routes, day, github, pulls_body};

const WARNING: &str = "Incomplete report: could not read a/b, e/f. Results include only accessible repositories. Spend-per-hour figures are unavailable until all selected repositories can be read. Check repository access or run analysis again to retry.";

#[tokio::test]
async fn unreadable_repositories_leave_a_warning_and_the_accessible_pulls() {
    let routes = Routes::default()
        .with("/repos/c/d/pulls", 200, &pulls_body(&[1, 2]))
        .with("/repos/g/h/pulls", 200, &pulls_body(&[3]));
    let github = github(routes);
    let repos: Vec<String> = ["a/b", "c/d", "e/f", "g/h"]
        .iter()
        .map(|repo| (*repo).to_owned())
        .collect();
    let batch = read_repositories(&github, &repos, day("2026-01-01"), day("2026-01-31"))
        .await
        .unwrap();
    assert_eq!(batch.warnings, vec![WARNING.to_owned()]);
    assert_eq!(batch.unavailable_repos, vec!["a/b", "e/f"]);
    assert_eq!(
        batch.stage,
        "Analysis complete with unavailable repositories"
    );
    let queued: Vec<(String, i64)> = batch
        .queue
        .iter()
        .map(|(repo, pull)| (repo.clone(), pull.number))
        .collect();
    assert_eq!(
        queued,
        vec![
            ("c/d".to_owned(), 1),
            ("c/d".to_owned(), 2),
            ("g/h".to_owned(), 3)
        ]
    );
}

#[tokio::test]
async fn readable_repositories_leave_no_warning_and_keep_the_selected_order() {
    let routes = Routes::default()
        .with("/repos/a/b/pulls", 200, &pulls_body(&[5]))
        .with("/repos/c/d/pulls", 200, &pulls_body(&[4]));
    let github = github(routes);
    let repos = vec!["c/d".to_owned(), "a/b".to_owned()];
    let batch = read_repositories(&github, &repos, day("2026-01-01"), day("2026-01-31"))
        .await
        .unwrap();
    assert!(batch.warnings.is_empty());
    assert!(batch.unavailable_repos.is_empty());
    assert_eq!(batch.stage, "Analysis complete");
    let queued: Vec<(&str, i64)> = batch
        .queue
        .iter()
        .map(|(repo, pull)| (repo.as_str(), pull.number))
        .collect();
    assert_eq!(queued, vec![("c/d", 4), ("a/b", 5)]);
}
