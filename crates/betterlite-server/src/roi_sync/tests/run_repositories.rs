#![cfg(test)]

use betterlite_roi::records::ROISyncPhase;

use super::support::{inputs, manager, report_of, routes_for, settings, settled};
use crate::roi_cache::error::ReportBlocked;
use crate::roi_cache::store::REPORT_PARAM;

#[tokio::test]
async fn run_reads_every_configured_repository() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12]);
    inputs.settings = settings(&["o/r", "c/d"]);
    inputs.routes = routes_for("o/r", &[12]).merge(routes_for("c/d", &[13]));
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let report = report_of(&inputs.repository);
    let numbers: Vec<i64> = report.pulls.iter().map(|pull| pull.number).collect();
    assert_eq!(numbers, vec![12, 13]);
    assert_eq!(report.repos, vec!["o/r".to_owned(), "c/d".to_owned()]);
    let status = manager.status();
    assert_eq!(status.phase, ROISyncPhase::Complete);
    assert_eq!(status.total, 2);
    assert_eq!(status.done, 2);
    let requested = inputs.routes.requested();
    assert!(requested.iter().any(|path| path == "/repos/o/r/pulls"));
    assert!(requested.iter().any(|path| path == "/repos/c/d/pulls"));
}

#[tokio::test]
async fn run_marks_the_repositories_it_cannot_read() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12]);
    inputs.settings = settings(&["o/r", "c/d"]);
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let report = report_of(&inputs.repository);
    assert_eq!(report.unavailable_repos, Some(vec!["c/d".to_owned()]));
    assert_eq!(report.warnings.as_deref().map(<[String]>::len), Some(1));
    assert_eq!(report.pulls.len(), 1);
    let status = manager.status();
    assert_eq!(status.phase, ROISyncPhase::Complete);
    assert_eq!(
        status.stage,
        "Analysis complete with unavailable repositories"
    );
}

#[tokio::test]
async fn run_fails_when_no_repository_can_be_read() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12]);
    inputs.settings = settings(&["x/y"]);
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let expected = ReportBlocked::NoRepositoryReadable.to_string();
    let status = manager.status();
    assert_eq!(status.phase, ROISyncPhase::Error);
    assert_eq!(status.error.as_deref(), Some(expected.as_str()));
    assert!(inputs.repository.written_named(REPORT_PARAM).is_none());
}

#[tokio::test]
async fn run_fails_when_the_accessible_repositories_have_no_pulls() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[]);
    inputs.settings = settings(&["o/r", "c/d"]);
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let expected = ReportBlocked::NoPullRequests {
        repos: "c/d".to_owned(),
    }
    .to_string();
    let status = manager.status();
    assert_eq!(status.phase, ROISyncPhase::Error);
    assert_eq!(status.error.as_deref(), Some(expected.as_str()));
    assert!(inputs.repository.written_named(REPORT_PARAM).is_none());
}
