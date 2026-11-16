#![cfg(test)]

use crate::roi_cache::repositories::read_repository;

use super::support::{Routes, day, github, pulls_body};

#[tokio::test]
async fn a_source_error_marks_the_repository_unavailable() {
    let routes = Routes::default().with("/repos/o/r/pulls", 403, "");
    let github = github(routes);
    let group = read_repository(&github, "o/r", day("2026-01-01"), day("2026-01-31")).await;
    assert_eq!(group.repo, "o/r");
    assert!(group.unavailable);
    assert!(group.pulls.is_empty());
}

#[tokio::test]
async fn a_readable_repository_returns_the_pulls_in_the_date_range() {
    let routes = Routes::default().with("/repos/o/r/pulls", 200, &pulls_body(&[3, 4]));
    let github = github(routes.clone());
    let group = read_repository(&github, "o/r", day("2026-01-05"), day("2026-01-05")).await;
    assert_eq!(group.repo, "o/r");
    assert!(!group.unavailable);
    let numbers: Vec<i64> = group.pulls.iter().map(|pull| pull.number).collect();
    assert_eq!(numbers, vec![3, 4]);
    assert_eq!(routes.requested(), vec!["/repos/o/r/pulls".to_owned()]);
}

#[tokio::test]
async fn the_date_range_reaches_the_pull_read() {
    let routes = Routes::default().with("/repos/o/r/pulls", 200, &pulls_body(&[3]));
    let github = github(routes);
    let group = read_repository(&github, "o/r", day("2026-02-01"), day("2026-02-28")).await;
    assert!(!group.unavailable);
    assert!(group.pulls.is_empty());
}
