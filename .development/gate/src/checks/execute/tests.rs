#![cfg(test)]

use std::fs;

use super::{execute, hints};
use crate::checks::Outcome;
use crate::git::git;
use crate::spec::command::{Command, Turn};

fn run_lock_probe(turn: Turn) -> Outcome {
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
        turn,
    };
    execute(root.path(), &log_dir, &probe, &[]).unwrap()
}

#[test]
fn a_one_at_a_time_check_runs_while_the_gate_holds_its_lock() {
    assert!(matches!(run_lock_probe(Turn::Alone), Outcome::Pass));
}

#[test]
fn other_checks_run_without_the_lock() {
    assert!(matches!(run_lock_probe(Turn::Build), Outcome::Fail { .. }));
}

#[test]
fn hints_name_the_failing_test_and_not_a_word_inside_another_word() {
    let output = "   Compiling thiserror-impl v2.0.21
   Compiling thiserror v2.0.21
     PASS [   0.007s] (31/73) betterlite-policy matcher::tests::a_child_whose_condition_fails
     FAIL [   0.009s] (34/73) betterlite-policy matcher::tests::keeps_attached_policies
thread 'keeps_attached_policies' panicked at crates/betterlite-policy/src/matcher/tests.rs:88:5:
     FAIL [   0.009s] (34/73) betterlite-policy matcher::tests::keeps_attached_policies
error: test run failed
";
    assert_eq!(
        hints(output),
        [
            "FAIL [   0.009s] (34/73) betterlite-policy matcher::tests::keeps_attached_policies",
            "thread 'keeps_attached_policies' panicked at crates/betterlite-policy/src/matcher/tests.rs:88:5:",
            "error: test run failed",
        ]
    );
}

#[test]
fn hints_keep_whole_words_and_fall_back_to_the_last_lines() {
    let compiler = "   Compiling betterlite v0.1.0\nerror[E0599]: no method named pick\n";
    assert_eq!(hints(compiler), ["error[E0599]: no method named pick"]);
    let quiet = "one\ntwo\nthree\nfour\n";
    assert_eq!(hints(quiet), ["two", "three", "four"]);
}

#[test]
fn hints_hand_on_the_findings_under_each_failed_hook() {
    let output = r#"rustfmt............................................................Passed
ruff...............................................................Failed
- hook id: ruff
- exit code: 1

  All checks passed!
  PERF403 Use a dictionary comprehension instead of a for-loop
    --> crates/betterlite-python/python/betterlite/fine_tuning/_calls.py:57:13
     |
  57 |             kept[name] = value
     |             ^^^^^^^^^^^^^^^^^^

  Found 1 error.

mypy strict........................................................Failed
- hook id: mypy
- exit code: 1

  crates/betterlite-python/tests/test_fine_tuning.py:28: error: Module "betterlite.fine_tuning.main" does not explicitly export attribute "HYPERPARAMETERS_DEFAULT"  [attr-defined]
  Found 1 error in 1 file (checked 6 source files)
basedpyright on the helper files the change touches...(no files to check)Skipped
"#;
    assert_eq!(
        hints(output),
        [
            "ruff...............................................................Failed",
            "  All checks passed!",
            "  PERF403 Use a dictionary comprehension instead of a for-loop",
            "    --> crates/betterlite-python/python/betterlite/fine_tuning/_calls.py:57:13",
            "     |",
            "  57 |             kept[name] = value",
            "     |             ^^^^^^^^^^^^^^^^^^",
            "  Found 1 error.",
            "mypy strict........................................................Failed",
            r#"  crates/betterlite-python/tests/test_fine_tuning.py:28: error: Module "betterlite.fine_tuning.main" does not explicitly export attribute "HYPERPARAMETERS_DEFAULT"  [attr-defined]"#,
            "  Found 1 error in 1 file (checked 6 source files)",
        ]
    );
}

#[test]
fn a_long_failed_hook_leaves_room_for_the_next_one() {
    let findings: Vec<String> = (0..30_u32)
        .map(|number| format!("  finding {number}"))
        .collect();
    let output = format!(
        "ast-grep rules......Failed\n{}\ntypos......Failed\n  crates/a.rs:1:1 teh\n",
        findings.join("\n")
    );
    let mut expected = vec!["ast-grep rules......Failed".to_owned()];
    expected.extend(findings.into_iter().take(20));
    expected.extend([
        "typos......Failed".to_owned(),
        "  crates/a.rs:1:1 teh".to_owned(),
    ]);
    assert_eq!(hints(&output), expected);
}
