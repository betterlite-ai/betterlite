#![cfg(test)]

use std::fs::{self, File};
use std::path::Path;

use crate::dispatch::progress::{Progress, clear};
use crate::dispatch::prompt::Brief;
use crate::dispatch::review::reviewer_rungs;
use crate::dispatch::vendor::mark;
use crate::dispatch::{
    EarlierReview, Places, climb_rung, find_runner, note_blocked, overflow_rung, run_ladder,
};
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
        rung: 0,
        rung_used: 0,
        limited: false,
        review_rounds: 0,
        blocked_reports: 0,
        last_blocked: 0,
        earlier: None,
        litellm: Vec::new(),
    }
}

#[test]
fn a_saved_progress_comes_back_in_place_of_the_notes() {
    let folder = tempfile::tempdir().unwrap();
    let mut saved = progress();
    saved.feedback = vec!["fix the parser".to_owned()];
    saved.number = 4;
    saved.used = 4;
    saved.rung = 1;
    saved.rung_used = 1;
    saved.review_rounds = 2;
    saved.earlier = Some(EarlierReview {
        commit: "abc123".to_owned(),
        findings: vec!["fix the parser".to_owned()],
    });
    saved.save(folder.path()).unwrap();
    let loaded = Progress::load(folder.path(), vec!["a new note".to_owned()]).unwrap();
    assert_eq!(loaded, saved);
}

#[test]
fn without_a_saved_progress_the_notes_start_the_feedback() {
    let folder = tempfile::tempdir().unwrap();
    let loaded = Progress::load(folder.path(), vec!["a note".to_owned()]).unwrap();
    let mut expected = progress();
    expected.feedback = vec!["a note".to_owned()];
    assert_eq!(loaded, expected);
}

#[test]
fn clearing_removes_the_saved_progress_and_needs_none() {
    let folder = tempfile::tempdir().unwrap();
    progress().save(folder.path()).unwrap();
    clear(folder.path()).unwrap();
    assert!(!folder.path().join("progress.json").exists());
    clear(folder.path()).unwrap();
}

#[test]
fn a_restarted_dispatch_goes_on_from_the_saved_rung() {
    let (agents, task) = fixtures();
    let brief = Brief {
        task: &task,
        file: "tasks/B000-workspace.yaml",
        branch: "task/B000-workspace",
        base: "abc123",
    };
    let folder = tempfile::tempdir().unwrap();
    // a dispatch that started over at the first rung would stop at these marks instead
    mark(
        folder.path(),
        "openai",
        "ERROR: You've hit your usage limit.",
    )
    .unwrap();
    mark(folder.path(), "anthropic", "You've hit your limit").unwrap();
    let places = Places {
        worktree: folder.path(),
        journal: folder.path(),
        git_common_dir: folder.path(),
        home: folder.path(),
        writable: &[],
    };
    let mut saved = progress();
    saved.feedback = vec!["the saved findings".to_owned()];
    saved.rung = agents.roles.worker.ladder.len();
    saved.save(folder.path()).unwrap();
    let error = run_ladder(&agents, &brief, &places, vec!["a new note".to_owned()]).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("the worker ladder ran out of attempts; last feedback: the saved findings")
    );
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

fn hold_seats(git: &Path, name: &str, count: u32) -> Vec<File> {
    let folder = git.join("betterlite/seats");
    fs::create_dir_all(&folder).unwrap();
    let mut held = Vec::new();
    for number in 1..=count {
        let seat = File::create(folder.join(format!("{name}-{number}"))).unwrap();
        seat.try_lock().unwrap();
        held.push(seat);
    }
    held
}

#[test]
fn a_try_whose_vendor_has_no_free_seat_goes_to_a_free_rung_of_another_vendor() {
    let (agents, _) = fixtures();
    let folder = tempfile::tempdir().unwrap();
    let places = Places {
        worktree: folder.path(),
        journal: folder.path(),
        git_common_dir: folder.path(),
        home: folder.path(),
        writable: &[],
    };
    let codex = find_runner(&agents, "codex-worker").unwrap();
    assert!(overflow_rung(&agents, &places, 0, codex).unwrap().is_none());
    let openai = hold_seats(folder.path(), "openai", 12);
    let (runner, rung) = overflow_rung(&agents, &places, 0, codex).unwrap().unwrap();
    assert_eq!(
        (runner.id.as_str(), rung.model.as_str()),
        ("claude-worker", "sonnet")
    );
    drop(openai);
}

#[test]
fn no_try_moves_while_the_other_vendor_is_full_or_at_its_limit() {
    let (agents, _) = fixtures();
    let folder = tempfile::tempdir().unwrap();
    let places = Places {
        worktree: folder.path(),
        journal: folder.path(),
        git_common_dir: folder.path(),
        home: folder.path(),
        writable: &[],
    };
    let codex = find_runner(&agents, "codex-worker").unwrap();
    let openai = hold_seats(folder.path(), "openai", 12);
    let writers = hold_seats(folder.path(), "anthropic-writer", 4);
    assert!(overflow_rung(&agents, &places, 0, codex).unwrap().is_none());
    drop(writers);
    mark(folder.path(), "anthropic", "You've hit your limit").unwrap();
    assert!(overflow_rung(&agents, &places, 0, codex).unwrap().is_none());
    drop(openai);
}
