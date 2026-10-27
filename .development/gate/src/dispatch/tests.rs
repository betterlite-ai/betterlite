#![cfg(test)]

use std::path::Path;

use crate::dispatch::prompt::Brief;
use crate::dispatch::{Progress, note_blocked};
use crate::spec::agents::Agents;
use crate::spec::task::Task;
use crate::yaml::read_yaml;

fn fixtures() -> (Agents, Task) {
    let development = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    (
        read_yaml(&development.join("agents.yaml")).unwrap(),
        read_yaml(&development.join("tasks/B000-workspace.yaml")).unwrap(),
    )
}

fn progress() -> Progress {
    Progress {
        feedback: Vec::new(),
        number: 0,
        review_rounds: 0,
        blocked_reports: 0,
        last_blocked: 0,
        earlier: None,
    }
}

#[test]
fn two_blocked_reports_in_a_row_go_back_to_the_architect() {
    let (agents, task) = fixtures();
    let brief = Brief {
        task: &task,
        file: "tasks/B000-workspace.yaml",
        branch: "task/B000-workspace",
        base: "abc123",
    };
    let mut progress = progress();
    progress.number = 1;
    note_blocked(&agents, &brief, &mut progress, vec!["scope".to_owned()]).unwrap();
    progress.number = 2;
    let error = note_blocked(
        &agents,
        &brief,
        &mut progress,
        vec!["scope again".to_owned()],
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("2 workers in a row reported blocked; last feedback: scope again")
    );
}

#[test]
fn a_blocked_report_after_another_outcome_starts_the_count_over() {
    let (agents, task) = fixtures();
    let brief = Brief {
        task: &task,
        file: "tasks/B000-workspace.yaml",
        branch: "task/B000-workspace",
        base: "abc123",
    };
    let mut progress = progress();
    progress.number = 1;
    note_blocked(&agents, &brief, &mut progress, vec!["first".to_owned()]).unwrap();
    progress.number = 3;
    note_blocked(&agents, &brief, &mut progress, vec!["third".to_owned()]).unwrap();
    assert_eq!(progress.blocked_reports, 1);
    assert_eq!(progress.feedback, vec!["third".to_owned()]);
}
