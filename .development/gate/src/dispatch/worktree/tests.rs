#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use tempfile::TempDir;

use super::{needs_own_gate, prepare};
use crate::git::{common_dir, coordinator, git};

fn fixture() -> (TempDir, PathBuf) {
    let folder = tempfile::tempdir().unwrap();
    let root = folder.path().join("main");
    fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]).unwrap();
    git(&root, &["config", "user.name", "Fixture"]).unwrap();
    git(&root, &["config", "user.email", "fixture@example.com"]).unwrap();
    git(&root, &["remote", "add", "origin", root.to_str().unwrap()]).unwrap();
    fs::write(root.join("counter"), "one\n").unwrap();
    git(&root, &["add", "counter"]).unwrap();
    git(&root, &["commit", "-qm", "initial counter"]).unwrap();
    (folder, root)
}

#[test]
fn migration_preserves_commits_staging_and_working_files() {
    let (folder, root) = fixture();
    let target = folder.path().join("B123");
    let branch = "task/B123-counter";
    git(
        &root,
        &["worktree", "add", "-qb", branch, target.to_str().unwrap()],
    )
    .unwrap();
    fs::write(target.join("counter"), "two\n").unwrap();
    git(&target, &["commit", "-qam", "second counter"]).unwrap();
    fs::write(target.join("counter"), "three\n").unwrap();
    git(&target, &["add", "counter"]).unwrap();
    fs::write(target.join("counter"), "four\n").unwrap();
    fs::write(target.join("notes"), "unfinished\n").unwrap();
    let status = ["status", "--porcelain=v1", "-z", "--untracked-files=all"];
    let before = git(&target, &status).unwrap();
    let head = git(&target, &["rev-parse", "HEAD"]).unwrap();
    prepare(&root, &target, branch).unwrap();
    assert!(target.join(".git").is_dir());
    assert_eq!(git(&target, &status).unwrap(), before);
    assert_eq!(git(&target, &["rev-parse", "HEAD"]).unwrap(), head);
    assert_eq!(git(&target, &["show", ":counter"]).unwrap(), "three\n");
    assert_eq!(coordinator(&target).unwrap(), common_dir(&root).unwrap());
    assert_ne!(common_dir(&target).unwrap(), common_dir(&root).unwrap());
    git(&root, &["gc", "--prune=now"]).unwrap();
    assert_eq!(git(&target, &["show", "HEAD:counter"]).unwrap(), "two\n");
}

#[test]
fn migration_preserves_local_excludes_with_staged_and_unstaged_files() {
    let (folder, root) = fixture();
    let target = folder.path().join("B123");
    let branch = "task/B123-counter";
    git(
        &root,
        &["worktree", "add", "-qb", branch, target.to_str().unwrap()],
    )
    .unwrap();
    let exclude = common_dir(&root).unwrap().join("info/exclude");
    fs::create_dir_all(exclude.parent().unwrap()).unwrap();
    fs::write(&exclude, "scratch-ignored/\n").unwrap();
    fs::write(target.join("counter"), "staged\n").unwrap();
    git(&target, &["add", "counter"]).unwrap();
    fs::write(target.join("counter"), "unstaged\n").unwrap();
    let artifact = target.join("scratch-ignored/artifact");
    fs::create_dir_all(artifact.parent().unwrap()).unwrap();
    fs::write(&artifact, "ignored\n").unwrap();
    let status = ["status", "--porcelain=v1", "-z", "--untracked-files=all"];
    let before = git(&target, &status).unwrap();
    let branch_head = git(&target, &["rev-parse", "HEAD"]).unwrap();

    prepare(&root, &target, branch).unwrap();

    assert_eq!(git(&target, &status).unwrap(), before);
    assert_eq!(git(&target, &["rev-parse", "HEAD"]).unwrap(), branch_head);
    assert_eq!(git(&target, &["show", ":counter"]).unwrap(), "staged\n");
    assert_eq!(
        fs::read_to_string(target.join("counter")).unwrap(),
        "unstaged\n"
    );
    assert_eq!(
        fs::read_to_string(target.join(".git/info/exclude")).unwrap(),
        "scratch-ignored/\n"
    );
    assert!(artifact.is_file());
}

#[test]
fn an_existing_task_branch_is_reused_in_a_private_checkout() {
    let (folder, root) = fixture();
    let target = folder.path().join("B123");
    let branch = "task/B123-counter";
    git(&root, &["switch", "-qc", branch]).unwrap();
    fs::write(root.join("counter"), "two\n").unwrap();
    git(&root, &["commit", "-qam", "second counter"]).unwrap();
    let head = git(&root, &["rev-parse", "HEAD"]).unwrap();
    git(&root, &["switch", "-q", "main"]).unwrap();
    prepare(&root, &target, branch).unwrap();
    assert_eq!(git(&target, &["rev-parse", "HEAD"]).unwrap(), head);
    assert_eq!(git(&target, &["config", "user.name"]).unwrap(), "Fixture\n");
    assert_eq!(
        git(&target, &["config", "maintenance.auto"]).unwrap(),
        "false\n"
    );
    assert_eq!(coordinator(&target).unwrap(), common_dir(&root).unwrap());
    assert!(prepare(&root, &target, "task/B124-other").is_err());
}

#[test]
fn a_checkout_builds_its_own_gate_only_without_a_shared_build_of_its_clean_gate() {
    let (folder, root) = fixture();
    let gate = root.join(".development/gate");
    fs::create_dir_all(&gate).unwrap();
    fs::write(gate.join("Cargo.toml"), "[package]\n").unwrap();
    git(&root, &["add", ".development/gate/Cargo.toml"]).unwrap();
    git(&root, &["commit", "-qm", "gate manifest"]).unwrap();
    let tree = git(&root, &["rev-parse", "HEAD:.development/gate"]).unwrap();
    let builds = folder.path().join("builds");
    assert!(needs_own_gate(&root, None).unwrap());
    assert!(needs_own_gate(&root, Some(&builds)).unwrap());
    fs::create_dir_all(builds.join(tree.trim())).unwrap();
    fs::write(builds.join(tree.trim()).join("gate"), "").unwrap();
    assert!(!needs_own_gate(&root, Some(&builds)).unwrap());
    fs::write(gate.join("Cargo.toml"), "[package]\nname = \"gate\"\n").unwrap();
    assert!(needs_own_gate(&root, Some(&builds)).unwrap());
}

#[test]
fn migration_refuses_a_checkout_used_by_a_process() {
    let (folder, root) = fixture();
    let target = folder.path().join("B123");
    let branch = "task/B123-counter";
    git(
        &root,
        &["worktree", "add", "-qb", branch, target.to_str().unwrap()],
    )
    .unwrap();
    let mut process = Command::new("sleep")
        .arg("10")
        .current_dir(&target)
        .spawn()
        .unwrap();
    let result = prepare(&root, &target, branch);
    process.kill().unwrap();
    process.wait().unwrap();
    assert!(result.unwrap_err().to_string().contains("still used"));
    assert!(target.join(".git").is_file());
}

#[test]
fn migration_refuses_an_unfinished_git_operation() {
    let (folder, root) = fixture();
    let target = folder.path().join("B123");
    let branch = "task/B123-counter";
    git(
        &root,
        &["worktree", "add", "-qb", branch, target.to_str().unwrap()],
    )
    .unwrap();
    let directory = git(&target, &["rev-parse", "--absolute-git-dir"]).unwrap();
    let marker = PathBuf::from(directory.trim()).join("MERGE_HEAD");
    let head = git(&target, &["rev-parse", "HEAD"]).unwrap();
    fs::write(&marker, &head).unwrap();
    let error = prepare(&root, &target, branch).unwrap_err();
    assert!(error.to_string().contains("unfinished Git operation"));
    assert!(target.join(".git").is_file());
    assert_eq!(fs::read_to_string(marker).unwrap(), head);
}

#[test]
fn a_task_checkout_runs_no_git_hooks() {
    let (folder, root) = fixture();
    let hooks = common_dir(&root).unwrap().join("hooks");
    fs::create_dir_all(&hooks).unwrap();
    fs::write(hooks.join("pre-commit"), "#!/bin/sh\nexit 1\n").unwrap();
    let target = folder.path().join("B123");
    let branch = "task/B123-counter";
    prepare(&root, &target, branch).unwrap();
    assert!(!target.join(".git/hooks/pre-commit").exists());
    fs::write(target.join(".git/hooks/pre-push"), "#!/bin/sh\nexit 1\n").unwrap();
    prepare(&root, &target, branch).unwrap();
    assert!(!target.join(".git/hooks/pre-push").exists());
    fs::write(target.join("counter"), "two\n").unwrap();
    git(&target, &["commit", "-qam", "second counter"]).unwrap();
}
