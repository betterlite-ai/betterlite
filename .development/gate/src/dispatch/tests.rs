#![cfg(test)]

use std::path::Path;

use crate::dispatch::prompt::Brief;
use crate::dispatch::review::reviewer_rungs;
use crate::dispatch::vendor::mark;
use crate::dispatch::{Places, Progress, climb_rung, find_runner, note_blocked};
use crate::spec::agents::Agents;
use crate::spec::task::{ReviewLevel, Task};
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
        used: 0,
        limited: false,
        review_rounds: 0,
        blocked_reports: 0,
        last_blocked: 0,
        earlier: None,
        litellm: Vec::new(),
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

#[test]
fn a_rung_whose_vendor_is_at_its_limit_is_skipped_without_a_run() {
    let (agents, task) = fixtures();
    let brief = Brief {
        task: &task,
        file: "tasks/B000-workspace.yaml",
        branch: "task/B000-workspace",
        base: "abc123",
    };
    let folder = tempfile::tempdir().unwrap();
    mark(
        folder.path(),
        "openai",
        "ERROR: You've hit your usage limit.",
    )
    .unwrap();
    let places = Places {
        worktree: folder.path(),
        journal: folder.path(),
        git_common_dir: folder.path(),
        home: folder.path(),
        writable: &[],
    };
    let mut progress = progress();
    let first = &agents.roles.worker.ladder[0];
    let lines = climb_rung(&agents, &brief, &places, first, &mut progress).unwrap();
    assert_eq!(lines, None);
    assert!(progress.limited);
    assert_eq!((progress.number, progress.used), (0, 0));
}

#[test]
fn no_rung_runs_once_the_worker_attempts_are_used_up() {
    let (agents, task) = fixtures();
    let brief = Brief {
        task: &task,
        file: "tasks/B000-workspace.yaml",
        branch: "task/B000-workspace",
        base: "abc123",
    };
    let folder = tempfile::tempdir().unwrap();
    let places = Places {
        worktree: folder.path(),
        journal: folder.path(),
        git_common_dir: folder.path(),
        home: folder.path(),
        writable: &[],
    };
    let mut progress = progress();
    progress.used = agents.policy.max_worker_attempts;
    for rung in &agents.roles.worker.ladder {
        let lines = climb_rung(&agents, &brief, &places, rung, &mut progress).unwrap();
        assert_eq!(lines, None);
    }
    assert_eq!(progress.number, 0);
    assert!(!progress.limited);
}

#[test]
fn the_reviewer_on_the_other_vendor_comes_first() {
    let (agents, _) = fixtures();
    let order = |worker: &str, level| {
        let runner = find_runner(&agents, worker).unwrap();
        reviewer_rungs(&agents, level, runner)
            .into_iter()
            .map(|rung| rung.model.clone())
            .collect::<Vec<String>>()
    };
    assert_eq!(
        order("codex-worker", ReviewLevel::Standard),
        ["sonnet", "gpt-6-astra"]
    );
    assert_eq!(
        order("claude-worker", ReviewLevel::Standard),
        ["gpt-6-astra", "sonnet"]
    );
    assert_eq!(
        order("claude-worker", ReviewLevel::Security),
        ["gpt-6-astra", "opus"]
    );
}
