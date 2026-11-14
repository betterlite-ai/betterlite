#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;

use crate::git::git;

fn reviewed_checkout() -> (TempDir, PathBuf, PathBuf, String) {
    let folder = tempfile::tempdir().unwrap();
    let worktree = folder.path().join("task");
    let journal = folder.path().join("journal");
    fs::create_dir_all(&worktree).unwrap();
    fs::create_dir_all(&journal).unwrap();
    git(&worktree, &["init", "-q", "-b", "main"]).unwrap();
    git(&worktree, &["config", "user.name", "Fixture"]).unwrap();
    git(&worktree, &["config", "user.email", "fixture@example.com"]).unwrap();
    git(
        &worktree,
        &["commit", "-q", "--allow-empty", "-m", "reviewed"],
    )
    .unwrap();
    let head = git(&worktree, &["rev-parse", "HEAD"]).unwrap();
    (folder, worktree, journal, head.trim().to_owned())
}

#[test]
fn ci_failures_reach_the_fix_turn_only_for_the_reviewed_head() {
    let (_folder, worktree, journal, head) = reviewed_checkout();
    let report = journal.join("ci-failures");
    assert!(super::take(&journal, &worktree).unwrap().is_empty());

    fs::write(
        &report,
        format!("{head}\nCI run 7 failed: FAIL task (3s)\n"),
    )
    .unwrap();
    assert_eq!(
        super::take(&journal, &worktree).unwrap(),
        vec!["CI run 7 failed: FAIL task (3s)".to_owned()]
    );
    assert!(!report.exists());

    fs::write(
        &report,
        "0000000000000000000000000000000000000000\nCI run 6 failed\n",
    )
    .unwrap();
    assert!(super::take(&journal, &worktree).unwrap().is_empty());
    assert!(!report.exists());
}

#[test]
fn ci_failures_reach_a_fix_turn_that_committed_on_top_of_the_reviewed_head() {
    let (_folder, worktree, journal, head) = reviewed_checkout();
    git(&worktree, &["commit", "-q", "--allow-empty", "-m", "fix"]).unwrap();
    fs::write(
        journal.join("ci-failures"),
        format!("{head}\nCI run 8 failed: FAIL task (4s)\n"),
    )
    .unwrap();
    assert_eq!(
        super::take(&journal, &worktree).unwrap(),
        vec!["CI run 8 failed: FAIL task (4s)".to_owned()]
    );
}
