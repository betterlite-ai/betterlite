#![cfg(test)]

use std::fs;

use super::execute;
use crate::checks::Outcome;
use crate::git::git;
use crate::spec::command::Command;

fn run_lock_probe(one_at_a_time: bool) -> Outcome {
    let root = tempfile::tempdir().unwrap();
    git(root.path(), &["init", "--quiet"]).unwrap();
    let log_dir = root.path().join(".git/betterlite/B000/gate");
    fs::create_dir_all(&log_dir).unwrap();
    let probe = Command {
        id: "deny".to_owned(),
        why: "the gate holds the deny lock while the check runs".to_owned(),
        run: "! flock --nonblock .git/betterlite/deny.lock true".to_owned(),
        when_exists: None,
        after_green: false,
        one_at_a_time,
    };
    execute(root.path(), &log_dir, &probe, &[]).unwrap()
}

#[test]
fn a_one_at_a_time_check_runs_while_the_gate_holds_its_lock() {
    assert!(matches!(run_lock_probe(true), Outcome::Pass));
}

#[test]
fn other_checks_run_without_the_lock() {
    assert!(matches!(run_lock_probe(false), Outcome::Fail { .. }));
}
