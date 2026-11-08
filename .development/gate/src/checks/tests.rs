#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::checks::{Outcome, Selection, history, render, run, selection};
use crate::development::Development;
use crate::facts::Facts;
use crate::git::git;
use crate::journal::journal_dir;
use crate::spec::backlog::Backlog;
use crate::spec::catalog::Catalog;
use crate::spec::command::{Command, Turn};
use crate::spec::task::Task;
use crate::yaml::read_yaml;

fn fixture() -> (tempfile::TempDir, Development, Task) {
    let folder = tempfile::tempdir().unwrap();
    let root = folder.path();
    git(root, &["init", "--quiet", "-b", "main"]).unwrap();
    git(
        root,
        &[
            "-c",
            "user.name=test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "--allow-empty",
            "-qm",
            "base",
        ],
    )
    .unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut catalog: Catalog = read_yaml(&source.join("checks.yaml")).unwrap();
    catalog.base.builtin.clear();
    catalog.base.commands.clear();
    let mut task: Task = read_yaml(&source.join("tasks/B000-workspace.yaml")).unwrap();
    task.checks.commands.clear();
    let development = Development {
        root: root.to_owned(),
        catalog,
        backlog: Backlog { tasks: Vec::new() },
    };
    (folder, development, task)
}

fn command(id: &str, script: &str) -> Command {
    Command {
        id: id.to_owned(),
        why: "test check".to_owned(),
        run: script.to_owned(),
        when_exists: None,
        after_green: false,
        turn: Turn::Build,
    }
}

#[test]
fn unknown_selected_and_skipped_checks_are_errors() {
    let (_folder, mut development, task) = fixture();
    development
        .catalog
        .base
        .commands
        .push(command("fmt", "true"));
    let selected = Selection {
        only: vec!["typo".to_owned()],
        skip: Vec::new(),
    };
    let skipped = Selection {
        only: Vec::new(),
        skip: vec!["typo".to_owned()],
    };
    assert!(run(&development, &task, &selected).is_err());
    assert!(run(&development, &task, &skipped).is_err());
}

#[test]
fn prerequisite_failure_prevents_expensive_work_but_focused_checks_still_run() {
    let (folder, mut development, task) = fixture();
    development
        .catalog
        .base
        .commands
        .extend([command("tests", "touch expensive"), command("fmt", "false")]);
    let verdict = run(&development, &task, &Selection::default()).unwrap();
    assert!(!verdict.passed);
    assert!(!folder.path().join("expensive").exists());
    let focused = Selection {
        only: vec!["tests".to_owned()],
        skip: Vec::new(),
    };
    assert!(run(&development, &task, &focused).unwrap().passed);
    assert!(folder.path().join("expensive").exists());
}

#[test]
fn selecting_no_checks_never_reports_success() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tasks/B000-workspace.yaml");
    let task: Task = read_yaml(&source).unwrap();
    let verdict = render(&task, "base", Path::new("logs"), Vec::new());
    assert!(!verdict.passed);
    assert_eq!(verdict.lines.last().unwrap(), "gate failed");
}

#[test]
fn a_failed_prerequisite_keeps_the_gate_red_when_work_is_skipped() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tasks/B000-workspace.yaml");
    let task: Task = read_yaml(&source).unwrap();
    let outcomes = vec![
        (
            "scope".to_owned(),
            0,
            Outcome::Fail {
                why: "outside scope".to_owned(),
                hints: Vec::new(),
            },
        ),
        (
            "tests".to_owned(),
            0,
            Outcome::Skip("fix prerequisites".to_owned()),
        ),
    ];
    assert!(!render(&task, "base", Path::new("logs"), outcomes).passed);
}

#[test]
fn a_new_run_without_a_result_or_an_error_is_unfinished() {
    let folder = tempfile::tempdir().unwrap();
    assert!(history::runs(folder.path()).unwrap().is_empty());
    let runs = folder.path().join("runs");
    fs::create_dir_all(runs.join("run-old")).unwrap();
    let earlier = history::runs(folder.path()).unwrap();
    for marker in ["result.json", "error.txt"] {
        let run = runs.join(format!("run-{marker}"));
        fs::create_dir_all(&run).unwrap();
        fs::write(run.join(marker), "{}").unwrap();
    }
    assert!(!history::unfinished(folder.path(), &earlier).unwrap());
    fs::create_dir_all(runs.join("run-cut")).unwrap();
    assert!(history::unfinished(folder.path(), &earlier).unwrap());
}

#[test]
fn the_local_gate_keeps_lint_checks_and_leaves_tests_to_ci() {
    let (_folder, mut development, mut task) = fixture();
    development.catalog.base.commands.extend([
        command("fmt", "true"),
        command("clippy", "true"),
        command("deny", "true"),
        command("mutants", "true"),
    ]);
    task.checks.commands.extend([
        command("route-tests", "true"),
        command("helper-python", "true"),
    ]);
    let selection = selection::builtins_and(&development, &task, selection::lint).unwrap();
    assert_eq!(selection.only, ["fmt", "clippy", "deny", "helper-python"]);
}

fn count_runs(root: &Path) -> usize {
    fs::read_to_string(root.join(".git/fmt-runs"))
        .unwrap()
        .lines()
        .count()
}

fn counted_fmt() -> (tempfile::TempDir, Development, Task) {
    let (folder, mut development, task) = fixture();
    development
        .catalog
        .base
        .commands
        .push(command("fmt", "echo run >> .git/fmt-runs"));
    assert!(
        run(&development, &task, &Selection::default())
            .unwrap()
            .passed
    );
    (folder, development, task)
}

#[test]
fn a_check_that_passed_on_the_same_clean_commit_and_base_is_not_run_again() {
    let (folder, development, task) = counted_fmt();
    let again = run(&development, &task, &Selection::default()).unwrap();
    assert!(again.passed);
    assert_eq!(count_runs(folder.path()), 1);
    assert!(
        again
            .lines
            .iter()
            .any(|line| line.starts_with("PASS fmt: passed on this commit and base in run-"))
    );
    let log_dir = journal_dir(folder.path(), &task.id).unwrap().join("gate");
    let mut facts = Facts::collect(folder.path()).unwrap();
    assert!(
        history::passed(&log_dir, &facts)
            .unwrap()
            .contains_key("fmt")
    );
    facts.base = "another-base".to_owned();
    assert!(history::passed(&log_dir, &facts).unwrap().is_empty());
}

#[test]
fn a_dirty_tree_or_a_new_commit_runs_the_check_again() {
    let (folder, development, task) = counted_fmt();
    let root = folder.path();
    fs::write(root.join("notes.txt"), "draft").unwrap();
    assert!(
        run(&development, &task, &Selection::default())
            .unwrap()
            .passed
    );
    assert_eq!(count_runs(root), 2);
    fs::remove_file(root.join("notes.txt")).unwrap();
    git(
        root,
        &[
            "-c",
            "user.name=test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "--allow-empty",
            "-qm",
            "next",
        ],
    )
    .unwrap();
    assert!(
        run(&development, &task, &Selection::default())
            .unwrap()
            .passed
    );
    assert_eq!(count_runs(root), 3);
}

#[test]
fn a_failed_check_runs_again_on_the_same_commit() {
    let (folder, mut development, task) = fixture();
    development
        .catalog
        .base
        .commands
        .push(command("fmt", "echo run >> .git/fmt-runs; false"));
    assert!(
        !run(&development, &task, &Selection::default())
            .unwrap()
            .passed
    );
    assert!(
        !run(&development, &task, &Selection::default())
            .unwrap()
            .passed
    );
    assert_eq!(count_runs(folder.path()), 2);
}
