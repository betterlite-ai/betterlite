#![cfg(test)]

use std::env;
use std::fs;
use std::path::PathBuf;

use crate::spec::backlog::Backlog;
use crate::spec::report::{Report, ReportStatus};
use crate::spec::task::Task;
use crate::yaml::read_yaml;

const TASK: &str = "id: B999
title: Probe
description: A task used by the format tests.
scope:
  paths: [crates/probe/**]
  lint_tables: false
references: []
dod:
  - text: The probe exists.
    proof: [scope]
checks:
  optional: []
  commands: []
oracle_cases: []
deviations: []
out_of_scope: []
pitfalls: []
review_level: standard
budget:
  max_changed_lines: 100
  max_new_expects: 0
";

fn written(name: &str, text: &str) -> PathBuf {
    let path = env::temp_dir().join(format!("betterlite-gate-{name}.yaml"));
    fs::write(&path, text).unwrap();
    path
}

#[test]
fn a_complete_task_reads() {
    let task: Task = read_yaml(&written("complete", TASK)).unwrap();
    assert_eq!(task.id, "B999");
    assert_eq!(task.scope.paths, ["crates/probe/**"]);
}

#[test]
fn an_unknown_field_is_rejected() {
    let text = format!("{TASK}notes: anything\n");
    assert!(
        read_yaml::<Task>(&written("unknown", &text))
            .is_err_and(|error| error.to_string().contains("notes"))
    );
}

#[test]
fn a_missing_field_is_rejected() {
    let text = TASK.replace("pitfalls: []\n", "");
    assert!(
        read_yaml::<Task>(&written("missing", &text))
            .is_err_and(|error| error.to_string().contains("pitfalls"))
    );
}

#[test]
fn a_duplicate_key_is_rejected() {
    let text = format!("{TASK}title: Again\n");
    assert!(
        read_yaml::<Task>(&written("duplicate", &text))
            .is_err_and(|error| error.to_string().contains("title"))
    );
}

#[test]
fn a_comment_is_rejected_because_it_can_cut_a_value_short() {
    let text = TASK.replace("title: Probe", "title: Probe #[expect(...)] cut here");
    assert!(
        read_yaml::<Task>(&written("comment", &text))
            .is_err_and(|error| error.to_string().contains("budget"))
    );
}

#[test]
fn an_anchor_is_rejected() {
    let text = TASK.replace("title: Probe", "title: &name Probe");
    assert!(
        read_yaml::<Task>(&written("anchor", &text))
            .is_err_and(|error| error.to_string().contains("budget"))
    );
}

#[test]
fn an_unknown_status_is_rejected() {
    let text = "tasks:\n  - id: B001\n    milestone: M0\n    title: Probe\n    status: started\n    depends_on: []\n    file: null\n";
    assert!(
        read_yaml::<Backlog>(&written("status", text))
            .is_err_and(|error| error.to_string().contains("started"))
    );
}

#[test]
fn a_report_reads_from_json() {
    let text = r#"{"task":"B004","status":"blocked","summary":"","blocker":"deny.toml is outside scope","prior_art":[],"deviations":[],"notes":[]}"#;
    let report: Report = serde_json::from_str(text).unwrap();
    assert_eq!(report.status, ReportStatus::Blocked);
}

#[test]
fn a_report_with_an_extra_field_is_rejected() {
    let text = r#"{"task":"B004","status":"done","summary":"x","blocker":"","prior_art":[],"deviations":[],"notes":[],"confidence":"high"}"#;
    assert!(
        serde_json::from_str::<Report>(text)
            .is_err_and(|error| error.to_string().contains("confidence"))
    );
}
