#![cfg(test)]

use std::path::Path;

use crate::dispatch::EarlierReview;
use crate::dispatch::prompt::{Brief, reviewer};
use crate::spec::task::Task;
use crate::yaml::read_yaml;

fn task() -> Task {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tasks/B000-workspace.yaml");
    read_yaml(&root).unwrap()
}

#[test]
fn a_first_review_reads_the_whole_branch() {
    let task = task();
    let brief = Brief {
        task: &task,
        file: "tasks/B000-workspace.yaml",
        branch: "task/B000-workspace",
        base: "abc123",
    };
    let prompt = reviewer(&brief, None);
    assert!(prompt.contains("git diff abc123...HEAD"));
    assert!(!prompt.contains("earlier round"));
}

#[test]
fn a_later_review_checks_the_earlier_findings_and_the_lines_changed_since() {
    let task = task();
    let brief = Brief {
        task: &task,
        file: "tasks/B000-workspace.yaml",
        branch: "task/B000-workspace",
        base: "abc123",
    };
    let earlier = EarlierReview {
        commit: "def456".to_owned(),
        findings: vec!["typed-errors at src/a.rs:3: boxed error Fix: use the enum".to_owned()],
    };
    let prompt = reviewer(&brief, Some(&earlier));
    assert!(prompt.contains("reviewed this branch at def456"));
    assert!(prompt.contains("- typed-errors at src/a.rs:3: boxed error Fix: use the enum"));
    assert!(prompt.contains("git diff def456...HEAD"));
}
