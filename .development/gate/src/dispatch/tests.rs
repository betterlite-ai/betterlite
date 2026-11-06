#![cfg(test)]

mod interrupted;

use std::fs::{self, File};
use std::ops::ControlFlow;
use std::path::Path;

use crate::dispatch::attempt::{UNFINISHED_NOTE, remove_extra_targets};
use crate::dispatch::chain::Chain;
use crate::dispatch::progress::{NextRun, Panel, Progress, Red, ReviewerPin, WorkerSession};
use crate::dispatch::prompt::Brief;
use crate::dispatch::review::{Bench, PARITY_RULES, review, reviewer_rungs};
use crate::dispatch::vendor::mark;
use crate::dispatch::{
    Attempt, BLOCKED_NOTE, EarlierReview, Failure, Places, Round, Step, apply_step, find_runner,
    note_blocked, run_ladder,
};
use crate::error::GateError;
use crate::git::git;
use crate::spec::agents::{Agents, OutputMode, Rung};
use crate::spec::review::ReviewVerdict;
use crate::spec::rules::Rules;
use crate::spec::task::{ReviewLevel, Task};
use crate::yaml::read_yaml;

fn fixtures() -> (Agents, Task) {
    let development = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut agents: Agents = read_yaml(&development.join("agents.yaml")).unwrap();
    let task: Task = read_yaml(&development.join("tasks/B000-workspace.yaml")).unwrap();
    agents.roles.worker.keep_rungs_for(&task.id);
    // these tests walk the four-rung ladder that ran before DeepSeek went in front
    agents
        .roles
        .worker
        .ladder
        .retain(|rung| rung.runner != "deepseek-worker");
    (agents, task)
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
        next: NextRun::Fresh,
        fix_turns: 0,
        granted: 0,
        reviewer: None,
        session: None,
        starts: Vec::new(),
        red: Vec::new(),
    }
}

#[test]
fn a_saved_progress_comes_back_with_the_new_notes_added() {
    let folder = tempfile::tempdir().unwrap();
    let mut saved = progress();
    saved.feedback = vec!["fix the parser".to_owned()];
    saved.number = 4;
    saved.used = 4;
    saved.rung = 1;
    saved.starts = vec![1, 1, 0, 0];
    saved.red = vec![Red::Check("tests".to_owned()), Red::Timeout];
    saved.review_rounds = 2;
    saved.earlier = Some(EarlierReview {
        commit: "abc123".to_owned(),
        findings: vec!["fix the parser".to_owned()],
    });
    saved.next = NextRun::Fix;
    saved.fix_turns = 1;
    saved.granted = 2;
    saved.reviewer = Some(ReviewerPin {
        runner: "claude-reviewer".to_owned(),
        model: "claude-opus-5-5".to_owned(),
        panel: Panel::Parity,
    });
    saved.save(folder.path()).unwrap();
    let notes = vec!["fix the parser".to_owned(), "a new note".to_owned()];
    let loaded = Progress::load(folder.path(), notes).unwrap();
    saved.feedback.push("a new note".to_owned());
    assert_eq!(loaded, saved);
}

#[test]
fn a_progress_saved_before_the_budgets_loads_with_their_defaults() {
    let folder = tempfile::tempdir().unwrap();
    fs::write(
        folder.path().join("progress.json"),
        r#"{"feedback":["a finding"],"number":3,"used":2,"rung":1,"rung_used":0,"limited":false,"review_rounds":4,"blocked_reports":0,"last_blocked":0,"earlier":null,"litellm":[]}"#,
    )
    .unwrap();
    let mut expected = progress();
    expected.feedback = vec!["a finding".to_owned()];
    expected.number = 3;
    expected.used = 2;
    expected.rung = 1;
    expected.review_rounds = 4;
    assert_eq!(Progress::load(folder.path(), Vec::new()).unwrap(), expected);
}

#[test]
fn a_progress_saved_before_starts_counts_every_rung_below_its_own_as_used() {
    let (agents, _) = fixtures();
    let ladder = &agents.roles.worker.ladder;
    let folder = tempfile::tempdir().unwrap();
    fs::write(
        folder.path().join("progress.json"),
        r#"{"feedback":[],"number":4,"used":4,"rung":2,"rung_used":1,"limited":false,"review_rounds":0,"blocked_reports":0,"last_blocked":0,"earlier":null,"litellm":[]}"#,
    )
    .unwrap();
    let mut loaded = Progress::load(folder.path(), Vec::new()).unwrap();
    loaded.fit(ladder);
    assert_eq!(
        loaded.starts,
        [ladder[0].attempts, ladder[1].attempts, 1, 0]
    );
    loaded.save(folder.path()).unwrap();
    let text = fs::read_to_string(folder.path().join("progress.json")).unwrap();
    assert!(!text.contains("rung_used"), "{text}");
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
fn a_worker_made_cargo_target_is_deleted_and_named() {
    let folder = tempfile::tempdir().unwrap();
    let target = folder.path().join("target");
    for name in ["b123-check", "llvm-cov-target"] {
        fs::create_dir_all(target.join(name).join("debug")).unwrap();
        fs::write(target.join(name).join("CACHEDIR.TAG"), "").unwrap();
    }
    fs::create_dir_all(target.join("debug")).unwrap();
    let notes = remove_extra_targets(folder.path()).unwrap();
    assert!(!target.join("b123-check").exists());
    assert!(target.join("llvm-cov-target").exists());
    assert!(target.join("debug").exists());
    assert_eq!(notes.len(), 1);
    assert!(notes[0].starts_with("The gate deleted target/b123-check,"));
    assert!(
        remove_extra_targets(&folder.path().join("missing"))
            .unwrap()
            .is_empty()
    );
}

fn trial_rung() -> Rung {
    Rung {
        runner: "claude-worker".to_owned(),
        model: "trial".to_owned(),
        effort: "high".to_owned(),
        attempts: 1,
        timeout_minutes: 1,
        tasks: Some(vec!["B9".to_owned()]),
    }
}

#[test]
fn a_rung_for_named_tasks_is_left_out_of_every_other_ladder() {
    let (mut trial, _) = fixtures();
    let (mut other, _) = fixtures();
    let general = other.roles.worker.ladder.len();
    trial.roles.worker.ladder.insert(0, trial_rung());
    other.roles.worker.ladder.insert(0, trial_rung());
    trial.roles.worker.keep_rungs_for("B9");
    other.roles.worker.keep_rungs_for("B000");
    assert_eq!(trial.roles.worker.ladder[0].model, "trial");
    assert_eq!(trial.roles.worker.ladder.len(), general.saturating_add(1));
    assert_eq!(other.roles.worker.ladder.len(), general);
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
    let mut loaded = Progress::load(folder.path(), vec!["a new note".to_owned()]).unwrap();
    let error = run_ladder(&agents, &brief, &places, &mut loaded, 0).unwrap_err();
    assert!(error.to_string().contains(
        "the worker ladder ran out of attempts; last feedback: the saved findings; a new note"
    ));
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
            .contains("2 blocked reports in a row; last feedback: scope again")
    );
}

#[test]
fn a_blocked_worker_resumes_its_session_once_then_stops_on_its_rung() {
    let (mut agents, task) = fixtures();
    claude_answers(
        &mut agents,
        r#"{"task":"B000","status":"blocked","summary":"Nothing is built yet, the implementation is still missing.","blocker":"implementation is still missing","owned":[],"prior_art":[],"notes":[]}"#,
    );
    let folder = tempfile::tempdir().unwrap();
    worktree(folder.path());
    let mut progress = progress();
    let error = run_ladder(
        &agents,
        &brief(&task),
        &places(folder.path()),
        &mut progress,
        0,
    )
    .unwrap_err();
    assert!(error.to_string().contains(
        "2 blocked reports in a row; last feedback: The last run reported blocked: implementation is still missing"
    ));
    let resumed = fs::read_to_string(folder.path().join("resumed")).unwrap();
    assert_eq!(resumed, "s1\n");
    let prompts = fs::read_to_string(folder.path().join("prompts")).unwrap();
    assert!(prompts.contains(BLOCKED_NOTE), "{prompts}");
    assert_eq!((progress.number, progress.used, progress.rung), (2, 1, 0));
    assert_eq!(progress.starts, [1, 0, 0, 0]);
    assert_eq!(progress.next, NextRun::Fresh);
}

#[test]
fn a_blocked_answer_while_its_own_gate_run_is_unfinished_is_not_counted() {
    let (mut agents, task) = fixtures();
    claude_answers(
        &mut agents,
        r#"{"task":"B000","status":"blocked","summary":"The preflight still waits for a build slot after many polls.","blocker":"preflight still running","owned":[],"prior_art":[],"notes":[]}"#,
    );
    for runner in &mut agents.runners {
        runner.args[1] = format!("mkdir -p '{{home}}/gate/runs/run-cut'; {}", runner.args[1]);
    }
    let folder = tempfile::tempdir().unwrap();
    worktree(folder.path());
    let mut progress = progress();
    let error = run_ladder(
        &agents,
        &brief(&task),
        &places(folder.path()),
        &mut progress,
        0,
    )
    .unwrap_err();
    assert!(error.to_string().contains("2 blocked reports in a row"));
    let resumed = fs::read_to_string(folder.path().join("resumed")).unwrap();
    assert_eq!(resumed, "s1\ns1\n");
    let prompts = fs::read_to_string(folder.path().join("prompts")).unwrap();
    assert!(prompts.contains(UNFINISHED_NOTE), "{prompts}");
    assert_eq!((progress.number, progress.used), (3, 1));
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
fn a_rung_pauses_without_a_run_while_every_vendor_it_can_use_is_at_its_limit() {
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
    mark(folder.path(), "anthropic", "You've hit your limit").unwrap();
    let places = Places {
        worktree: folder.path(),
        journal: folder.path(),
        git_common_dir: folder.path(),
        home: folder.path(),
        writable: &[],
    };
    let mut progress = progress();
    let error = run_ladder(&agents, &brief, &places, &mut progress, 0).unwrap_err();
    assert!(matches!(error, GateError::Unavailable { .. }));
    assert!(progress.limited);
    assert_eq!((progress.number, progress.used), (0, 0));
}

#[test]
fn a_limited_vendor_sends_the_start_to_the_other_vendor_at_the_same_level() {
    let (agents, _) = fixtures();
    let folder = tempfile::tempdir().unwrap();
    let chain = Chain::new(&agents).unwrap();
    let mut progress = progress();
    progress.fit(&agents.roles.worker.ladder);
    let pick = |own: usize| {
        chain
            .pick(&places(folder.path()), &progress, &chain.links[own])
            .unwrap()
            .map(|link| link.rung.model.clone())
    };
    assert_eq!(pick(0).as_deref(), Some("gpt-6-luna"));
    mark(
        folder.path(),
        "openai",
        "ERROR: You've hit your usage limit.",
    )
    .unwrap();
    assert_eq!(pick(0).as_deref(), Some("claude-sonnet-5-5"));
    assert_eq!(pick(3).as_deref(), Some("claude-opus-5-5"));
    mark(folder.path(), "anthropic", "You've hit your limit").unwrap();
    assert_eq!(pick(3), None);
}

#[test]
fn a_limited_claude_rung_runs_on_the_open_codex_rung_at_its_level() {
    let (agents, _) = fixtures();
    let folder = tempfile::tempdir().unwrap();
    mark(folder.path(), "anthropic", "You've hit your limit").unwrap();
    let chain = Chain::new(&agents).unwrap();
    let mut progress = progress();
    progress.fit(&agents.roles.worker.ladder);
    progress.starts = vec![1, 0, 0, 0];
    let pick = |own: usize| {
        chain
            .pick(&places(folder.path()), &progress, &chain.links[own])
            .unwrap()
            .map(|link| link.rung.model.clone())
    };
    assert_eq!(pick(1).as_deref(), Some("gpt-6-astra"));
    assert_eq!(pick(2).as_deref(), Some("gpt-6-astra"));
}

#[test]
fn a_failed_start_stays_on_its_rung_then_takes_the_next_one() {
    let (agents, _) = fixtures();
    let chain = Chain::new(&agents).unwrap();
    let mut progress = progress();
    progress.fit(&agents.roles.worker.ladder);
    progress.starts = vec![1, 0, 0, 0];
    assert_eq!(chain.next_rung(&progress, false), 1);
    progress.rung = 1;
    progress.starts = vec![1, 1, 0, 0];
    assert_eq!(chain.next_rung(&progress, false), 1);
    assert_eq!(chain.next_rung(&progress, true), 2);
    progress.rung = 3;
    progress.starts = vec![1, 1, 1, 1];
    assert_eq!(chain.next_rung(&progress, false), chain.links.len());
    // while Claude was at its limit Sonnet's start ran on gpt-6-astra, so Sonnet still gets its turn
    progress.starts = vec![1, 0, 0, 1];
    assert_eq!(chain.next_rung(&progress, true), 1);
    progress.rung = 0;
    progress.starts = vec![0, 0, 0, 0];
    assert_eq!(chain.next_rung(&progress, true), 1);
}

#[test]
fn the_same_red_at_the_end_of_two_starts_moves_the_task_to_the_next_rung() {
    let (agents, task) = fixtures();
    let folder = tempfile::tempdir().unwrap();
    let chain = Chain::new(&agents).unwrap();
    let mut progress = progress();
    progress.fit(&agents.roles.worker.ladder);
    progress.rung = 1;
    progress.starts = vec![1, 1, 0, 0];
    progress.red = vec![Red::Check("tests".to_owned())];
    let retry = |check: &str| {
        Step::Retry(Failure {
            feedback: vec![format!("{check} is red")],
            red: vec![Red::Check(check.to_owned())],
        })
    };
    for (check, rung) in [("clippy", 1), ("clippy", 2)] {
        let flow = apply_step(
            &chain,
            &brief(&task),
            &places(folder.path()),
            &mut progress,
            retry(check),
        )
        .unwrap();
        assert_eq!(flow, ControlFlow::Continue(()));
        assert_eq!(progress.rung, rung);
    }
    assert_eq!(progress.red, [Red::Check("clippy".to_owned())]);
    assert_eq!(progress.feedback, ["clippy is red".to_owned()]);
}

#[test]
fn a_dispatch_stopped_by_a_usage_limit_keeps_its_place() {
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
    saved.number = 2;
    saved.used = 2;
    saved.rung = 1;
    saved.starts = vec![1, 1, 0, 0];
    saved.save(folder.path()).unwrap();
    let mut loaded = Progress::load(folder.path(), Vec::new()).unwrap();
    let error = run_ladder(&agents, &brief, &places, &mut loaded, 0).unwrap_err();
    assert!(matches!(error, GateError::Unavailable { .. }));
    assert!(error.to_string().contains(
        "every runner left on the worker ladder is unavailable (openai is at its usage limit: ERROR: You've hit your usage limit.; anthropic is at its usage limit: You've hit your limit); last feedback: the saved findings"
    ));
    saved.limited = true;
    assert_eq!(loaded, saved);
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
    let error = run_ladder(&agents, &brief, &places, &mut progress, 0).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("the worker ladder ran out of attempts")
    );
    assert_eq!(progress.number, 0);
    assert!(!progress.limited);
}

#[test]
fn the_reviewer_on_the_other_vendor_comes_first() {
    let (agents, _) = fixtures();
    let order = |worker: &str, level| {
        let runner = find_runner(&agents, worker).unwrap();
        let bench = Bench {
            panel: Panel::Standard,
            pinned: None,
        };
        reviewer_rungs(&agents, level, bench, runner)
            .into_iter()
            .map(|rung| rung.model.clone())
            .collect::<Vec<String>>()
    };
    assert_eq!(
        order("codex-worker", ReviewLevel::Standard),
        ["claude-sonnet-5-5", "gpt-6-astra"]
    );
    assert_eq!(
        order("claude-worker", ReviewLevel::Standard),
        ["gpt-6-astra", "claude-sonnet-5-5"]
    );
    assert_eq!(
        order("claude-worker", ReviewLevel::Security),
        ["gpt-6-astra", "claude-opus-5-5"]
    );
}

#[test]
fn the_reviewer_of_the_last_round_comes_first() {
    let (agents, _) = fixtures();
    let codex = find_runner(&agents, "codex-worker").unwrap();
    let order = |runner: &str, model: &str| {
        let pin = ReviewerPin {
            runner: runner.to_owned(),
            model: model.to_owned(),
            panel: Panel::Standard,
        };
        let bench = Bench {
            panel: Panel::Standard,
            pinned: Some(&pin),
        };
        reviewer_rungs(&agents, ReviewLevel::Standard, bench, codex)
            .into_iter()
            .map(|rung| rung.model.clone())
            .collect::<Vec<String>>()
    };
    assert_eq!(
        order("codex-reviewer", "gpt-6-astra"),
        ["gpt-6-astra", "claude-sonnet-5-5"]
    );
    assert_eq!(
        order("claude-reviewer", "claude-sonnet-5-5"),
        ["claude-sonnet-5-5", "gpt-6-astra"]
    );
    // a pin whose rung left agents.yaml falls back to the usual order
    assert_eq!(
        order("codex-reviewer", "gpt-retired"),
        ["claude-sonnet-5-5", "gpt-6-astra"]
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
fn a_start_whose_vendor_has_no_free_seat_goes_to_the_other_vendor_at_its_level() {
    let (agents, _) = fixtures();
    let folder = tempfile::tempdir().unwrap();
    let chain = Chain::new(&agents).unwrap();
    let mut progress = progress();
    progress.fit(&agents.roles.worker.ladder);
    let pick = |own: usize| {
        chain
            .pick(&places(folder.path()), &progress, &chain.links[own])
            .unwrap()
            .map(|link| link.rung.model.clone())
    };
    assert_eq!(pick(0).as_deref(), Some("gpt-6-luna"));
    let openai = hold_seats(folder.path(), "openai", 12);
    assert_eq!(pick(0).as_deref(), Some("claude-sonnet-5-5"));
    assert_eq!(pick(3).as_deref(), Some("claude-opus-5-5"));
    drop(openai);
}

#[test]
fn no_start_moves_while_the_other_vendor_is_full_or_at_its_limit() {
    let (agents, _) = fixtures();
    let folder = tempfile::tempdir().unwrap();
    let chain = Chain::new(&agents).unwrap();
    let mut progress = progress();
    progress.fit(&agents.roles.worker.ladder);
    let pick = || {
        chain
            .pick(&places(folder.path()), &progress, &chain.links[0])
            .unwrap()
            .map(|link| link.rung.model.clone())
    };
    let openai = hold_seats(folder.path(), "openai", 12);
    // a worker start takes a writer seat first, and Claude has 12 of those beside 16 run seats
    let anthropic = hold_seats(folder.path(), "anthropic-writer", 12);
    assert_eq!(pick().as_deref(), Some("gpt-6-luna"));
    drop(anthropic);
    mark(folder.path(), "anthropic", "You've hit your limit").unwrap();
    assert_eq!(pick().as_deref(), Some("gpt-6-luna"));
    drop(openai);
}

fn brief(task: &Task) -> Brief<'_> {
    Brief {
        task,
        file: "tasks/B000-workspace.yaml",
        branch: "task/B000-workspace",
        base: "abc123",
    }
}

fn places(folder: &Path) -> Places<'_> {
    Places {
        worktree: folder,
        journal: folder,
        git_common_dir: folder,
        home: folder,
        writable: &[],
    }
}

fn pin() -> ReviewerPin {
    ReviewerPin {
        runner: "claude-reviewer".to_owned(),
        model: "claude-sonnet-5-5".to_owned(),
        panel: Panel::Standard,
    }
}

fn reviewed(verdict: ReviewVerdict, findings: &[&str]) -> Step {
    Step::Reviewed(Round {
        verdict,
        review: EarlierReview {
            commit: "def456".to_owned(),
            findings: findings.iter().map(|&line| line.to_owned()).collect(),
        },
        reviewer: pin(),
        litellm: vec!["a LiteLLM bug".to_owned()],
    })
}

#[test]
fn a_red_gate_gets_two_fix_turns_then_a_fresh_start() {
    let (agents, task) = fixtures();
    let folder = tempfile::tempdir().unwrap();
    let mut progress = progress();
    for expected in [(1, NextRun::Fix), (2, NextRun::Fix), (0, NextRun::Fresh)] {
        let step = Step::Fix(Failure {
            feedback: vec!["a red check".to_owned()],
            red: vec![Red::Check("tests".to_owned())],
        });
        let flow = apply_step(
            &Chain::new(&agents).unwrap(),
            &brief(&task),
            &places(folder.path()),
            &mut progress,
            step,
        )
        .unwrap();
        assert_eq!(flow, ControlFlow::Continue(()));
        assert_eq!((progress.fix_turns, progress.next), expected);
    }
    assert_eq!(progress.feedback, vec!["a red check".to_owned()]);
}

#[test]
fn a_run_without_an_answer_starts_fresh_on_the_same_rung() {
    let (agents, task) = fixtures();
    let folder = tempfile::tempdir().unwrap();
    let mut progress = progress();
    progress.next = NextRun::Fix;
    progress.fix_turns = 1;
    let step = Step::Retry(Failure {
        feedback: vec!["no answer".to_owned()],
        red: vec![Red::Silent],
    });
    let flow = apply_step(
        &Chain::new(&agents).unwrap(),
        &brief(&task),
        &places(folder.path()),
        &mut progress,
        step,
    )
    .unwrap();
    assert_eq!(flow, ControlFlow::Continue(()));
    assert_eq!((progress.fix_turns, progress.next), (0, NextRun::Fresh));
    assert_eq!(progress.feedback, vec!["no answer".to_owned()]);
}

#[test]
fn a_review_with_changes_keeps_its_findings_and_its_reviewer() {
    let (agents, task) = fixtures();
    let folder = tempfile::tempdir().unwrap();
    let mut progress = progress();
    progress.fix_turns = 2;
    let step = reviewed(ReviewVerdict::Changes, &["a finding"]);
    let flow = apply_step(
        &Chain::new(&agents).unwrap(),
        &brief(&task),
        &places(folder.path()),
        &mut progress,
        step,
    )
    .unwrap();
    assert_eq!(flow, ControlFlow::Continue(()));
    assert_eq!(progress.review_rounds, 1);
    assert_eq!((progress.fix_turns, progress.next), (0, NextRun::Fix));
    assert_eq!(progress.feedback, vec!["a finding".to_owned()]);
    assert_eq!(
        progress.earlier,
        Some(EarlierReview {
            commit: "def456".to_owned(),
            findings: vec!["a finding".to_owned()],
        })
    );
    assert_eq!(progress.reviewer, Some(pin()));
    assert_eq!(progress.litellm, vec!["a LiteLLM bug".to_owned()]);
}

#[test]
fn the_last_review_round_stops_the_task_with_its_findings_kept() {
    let (agents, task) = fixtures();
    let folder = tempfile::tempdir().unwrap();
    let mut progress = progress();
    progress.review_rounds = agents.policy.max_review_rounds;
    let step = reviewed(ReviewVerdict::Changes, &["still wrong"]);
    let error = apply_step(
        &Chain::new(&agents).unwrap(),
        &brief(&task),
        &places(folder.path()),
        &mut progress,
        step,
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("review rounds ran out: still wrong")
    );
    assert_eq!(progress.review_rounds, agents.policy.max_review_rounds);
    assert_eq!(progress.feedback, vec!["still wrong".to_owned()]);
    assert_eq!((progress.fix_turns, progress.next), (0, NextRun::Fix));
}

#[test]
fn an_approval_clears_the_feedback_and_keeps_the_approved_commit() {
    let (agents, task) = fixtures();
    let folder = tempfile::tempdir().unwrap();
    let mut progress = progress();
    progress.feedback = vec!["an old finding".to_owned()];
    progress.litellm = vec!["an earlier LiteLLM bug".to_owned()];
    let step = reviewed(ReviewVerdict::Approve, &[]);
    let flow = apply_step(
        &Chain::new(&agents).unwrap(),
        &brief(&task),
        &places(folder.path()),
        &mut progress,
        step,
    )
    .unwrap();
    let ready = format!(
        "B000 is ready to merge: branch task/B000-workspace in {}",
        folder.path().display()
    );
    assert_eq!(
        flow,
        ControlFlow::Break(vec![
            ready,
            "an earlier LiteLLM bug".to_owned(),
            "a LiteLLM bug".to_owned(),
        ])
    );
    assert!(progress.feedback.is_empty());
    assert!(progress.litellm.is_empty());
    assert_eq!(
        progress.earlier,
        Some(EarlierReview {
            commit: "def456".to_owned(),
            findings: Vec::new(),
        })
    );
    assert_eq!(progress.next, NextRun::Fix);
}

#[test]
fn a_grant_on_a_used_up_ladder_goes_back_to_its_last_rung() {
    let (agents, _) = fixtures();
    let ladder = &agents.roles.worker.ladder;
    let last = ladder.len() - 1;
    let mut progress = progress();
    progress.rung = ladder.len();
    progress.starts = ladder.iter().map(|rung| rung.attempts).collect();
    progress.grant(ladder, 0);
    assert_eq!((progress.rung, progress.granted), (ladder.len(), 0));
    assert!(!progress.open(ladder, last));
    progress.grant(ladder, 2);
    assert_eq!((progress.rung, progress.granted), (last, 2));
    assert!(progress.open(ladder, last));
    progress.rung = ladder.len();
    progress.grant(ladder, 1);
    assert_eq!((progress.rung, progress.granted), (last, 3));
    let mut climbing = self::progress();
    climbing.rung = 1;
    climbing.grant(ladder, 1);
    assert_eq!((climbing.rung, climbing.granted), (1, 1));
}

fn answer_with(agents: &mut Agents, answer: &str) {
    for runner in &mut agents.runners {
        "sh".clone_into(&mut runner.program);
        runner.args = vec![
            "-c".to_owned(),
            format!("cat > /dev/null; printf '{answer}' > '{{output}}'"),
        ];
        runner.output = OutputMode::File;
    }
}

fn worktree(folder: &Path) {
    git(folder, &["init", "--quiet"]).unwrap();
    git(
        folder,
        &[
            "-c",
            "user.name=gate",
            "-c",
            "user.email=gate@example.com",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "--no-verify",
            "--quiet",
            "-m",
            "start",
        ],
    )
    .unwrap();
    let schema = folder.join(".development/schema");
    fs::create_dir_all(&schema).unwrap();
    let development = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    fs::copy(
        development.join("schema/report.schema.json"),
        schema.join("report.schema.json"),
    )
    .unwrap();
}

#[test]
fn a_fix_turn_does_not_count_as_a_worker_attempt() {
    let (mut agents, task) = fixtures();
    answer_with(&mut agents, "not a report");
    let folder = tempfile::tempdir().unwrap();
    worktree(folder.path());
    let mut progress = progress();
    let error = run_ladder(
        &agents,
        &brief(&task),
        &places(folder.path()),
        &mut progress,
        0,
    )
    .unwrap_err();
    assert!(error.to_string().contains("ran out of attempts"), "{error}");
    // the same red every time gives each rung one start
    let runs = 4 * agents.policy.max_fix_turns.saturating_add(1);
    assert_eq!((progress.number, progress.used), (runs, 4));
    assert_eq!(progress.starts, vec![1, 1, 1, 1]);
    assert!(progress.feedback[0].starts_with("The final answer did not match the report format"));
}

#[test]
fn a_granted_start_runs_on_the_last_rung_once_the_attempts_ran_out() {
    let (mut agents, task) = fixtures();
    answer_with(&mut agents, "not a report");
    let folder = tempfile::tempdir().unwrap();
    worktree(folder.path());
    let ladder = &agents.roles.worker.ladder;
    let last = ladder.len() - 1;
    let mut progress = progress();
    progress.used = agents.policy.max_worker_attempts;
    progress.rung = last;
    progress.starts = ladder.iter().map(|rung| rung.attempts).collect();
    run_ladder(
        &agents,
        &brief(&task),
        &places(folder.path()),
        &mut progress,
        1,
    )
    .unwrap_err();
    assert_eq!(
        (progress.number, progress.used, progress.starts[last]),
        (
            agents.policy.max_fix_turns + 1,
            agents.policy.max_worker_attempts + 1,
            ladder[last].attempts + 1
        )
    );
}

#[test]
fn a_granted_start_runs_on_a_stand_in_while_the_last_rung_vendor_is_limited() {
    let (mut agents, task) = fixtures();
    answer_with(&mut agents, "not a report");
    let folder = tempfile::tempdir().unwrap();
    worktree(folder.path());
    mark(
        folder.path(),
        "openai",
        "ERROR: You've hit your usage limit.",
    )
    .unwrap();
    let chain = Chain::new(&agents).unwrap();
    let ladder = &agents.roles.worker.ladder;
    let last = ladder.len() - 1;
    let mut progress = progress();
    progress.used = agents.policy.max_worker_attempts;
    progress.rung = last;
    progress.starts = ladder.iter().map(|rung| rung.attempts).collect();
    let places = places(folder.path());
    let stand_in = chain.pick(&places, &progress, &chain.links[last]).unwrap();
    assert!(stand_in.is_none());
    run_ladder(&agents, &brief(&task), &places, &mut progress, 2).unwrap_err();
    assert_eq!(ladder[last - 1].model, "claude-opus-5-5");
    assert_eq!(
        (
            progress.used,
            progress.starts[last - 1],
            progress.starts[last]
        ),
        (
            agents.policy.max_worker_attempts + 2,
            ladder[last - 1].attempts + 2,
            ladder[last].attempts
        )
    );
}

fn claude_answers(agents: &mut Agents, report: &str) {
    let answer = format!(r#"printf '{{"structured_output":{report},"session_id":"s1"}}'"#);
    for runner in &mut agents.runners {
        "sh".clone_into(&mut runner.program);
        runner.args = vec!["-c".to_owned(), format!("cat > /dev/null; {answer}")];
        runner.resume = Some(vec![
            "-c".to_owned(),
            format!("echo {{session}} >> '{{home}}/resumed'; cat >> '{{home}}/prompts'; {answer}"),
        ]);
        runner.output = OutputMode::ClaudeJson;
    }
}

#[test]
fn fix_turns_go_on_in_the_session_of_the_run_before_them() {
    let (mut agents, task) = fixtures();
    claude_answers(&mut agents, "{}");
    agents.roles.worker.ladder.truncate(1);
    let folder = tempfile::tempdir().unwrap();
    worktree(folder.path());
    let mut progress = progress();
    run_ladder(
        &agents,
        &brief(&task),
        &places(folder.path()),
        &mut progress,
        0,
    )
    .unwrap_err();
    let first = &agents.roles.worker.ladder[0];
    let fix_turns = first.attempts.saturating_mul(agents.policy.max_fix_turns);
    let resumed = fs::read_to_string(folder.path().join("resumed")).unwrap();
    assert_eq!(resumed, "s1\n".repeat(usize::try_from(fix_turns).unwrap()));
    let prompts = fs::read_to_string(folder.path().join("prompts")).unwrap();
    assert_eq!(
        prompts.matches("Your last run did not pass.").count(),
        usize::try_from(fix_turns).unwrap()
    );
    assert!(!prompts.contains("You are the betterlite worker"));
    let session = WorkerSession {
        runner: first.runner.clone(),
        model: first.model.clone(),
        id: "s1".to_owned(),
    };
    assert_eq!(progress.session, Some(session));
}

#[test]
fn a_session_from_another_model_starts_a_new_one() {
    let (mut agents, task) = fixtures();
    claude_answers(&mut agents, "{}");
    agents.roles.worker.ladder.truncate(1);
    let folder = tempfile::tempdir().unwrap();
    worktree(folder.path());
    let mut progress = progress();
    progress.next = NextRun::Fix;
    progress.feedback = vec!["a red check".to_owned()];
    progress.session = Some(WorkerSession {
        runner: agents.roles.worker.ladder[0].runner.clone(),
        model: "another".to_owned(),
        id: "s0".to_owned(),
    });
    run_ladder(
        &agents,
        &brief(&task),
        &places(folder.path()),
        &mut progress,
        0,
    )
    .unwrap_err();
    let resumed = fs::read_to_string(folder.path().join("resumed")).unwrap();
    assert!(!resumed.contains("s0"), "{resumed}");
}

#[test]
fn a_pin_saved_before_the_parity_pass_loads_as_standard() {
    let folder = tempfile::tempdir().unwrap();
    fs::write(
        folder.path().join("progress.json"),
        r#"{"feedback":[],"number":1,"used":1,"rung":0,"rung_used":1,"limited":false,"review_rounds":1,"blocked_reports":0,"last_blocked":0,"earlier":null,"litellm":[],"reviewer":{"runner":"claude-reviewer","model":"claude-sonnet-5-5"}}"#,
    )
    .unwrap();
    let loaded = Progress::load(folder.path(), Vec::new()).unwrap();
    assert_eq!(loaded.reviewer, Some(pin()));
}

#[test]
fn the_parity_ladder_ignores_a_standard_pin() {
    let (agents, _) = fixtures();
    let claude = find_runner(&agents, "claude-worker").unwrap();
    let order = |pinned: Option<&ReviewerPin>| {
        let bench = Bench {
            panel: Panel::Parity,
            pinned,
        };
        reviewer_rungs(&agents, ReviewLevel::Standard, bench, claude)
            .into_iter()
            .map(|rung| rung.model.clone())
            .collect::<Vec<String>>()
    };
    assert_eq!(order(None), ["gpt-6-astra", "claude-opus-5-5"]);
    let standard = ReviewerPin {
        runner: "claude-reviewer".to_owned(),
        model: "claude-opus-5-5".to_owned(),
        panel: Panel::Standard,
    };
    assert_eq!(order(Some(&standard)), ["gpt-6-astra", "claude-opus-5-5"]);
    let parity = ReviewerPin {
        panel: Panel::Parity,
        ..standard
    };
    assert_eq!(order(Some(&parity)), ["claude-opus-5-5", "gpt-6-astra"]);
}

#[test]
fn the_parity_pass_counts_its_review_rounds_from_zero() {
    let (agents, task) = fixtures();
    let folder = tempfile::tempdir().unwrap();
    let mut progress = progress();
    progress.review_rounds = agents.policy.max_review_rounds;
    progress.reviewer = Some(pin());
    for expected in [1, 2] {
        let step = Step::Reviewed(Round {
            verdict: ReviewVerdict::Changes,
            review: EarlierReview {
                commit: "def456".to_owned(),
                findings: vec!["a parity finding".to_owned()],
            },
            reviewer: ReviewerPin {
                panel: Panel::Parity,
                ..pin()
            },
            litellm: Vec::new(),
        });
        let flow = apply_step(
            &Chain::new(&agents).unwrap(),
            &brief(&task),
            &places(folder.path()),
            &mut progress,
            step,
        )
        .unwrap();
        assert_eq!(flow, ControlFlow::Continue(()));
        assert_eq!(progress.review_rounds, expected);
    }
}

#[test]
fn the_parity_rules_are_reviewed_rules() {
    let development = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let rules: Rules = read_yaml(&development.join("rules.yaml")).unwrap();
    for id in PARITY_RULES {
        assert!(
            rules
                .rules
                .iter()
                .any(|rule| rule.id == id && rule.reviewed),
            "{id}"
        );
    }
}

const APPROVE: &str = r#"{"task":"B000","verdict":"approve","findings":[]}"#;

fn review_answers(agents: &mut Agents, parity: &str) {
    for runner in &mut agents.runners {
        "sh".clone_into(&mut runner.program);
        runner.args = vec![
            "-c".to_owned(),
            format!(
                "cat > /dev/null; basename '{{output}}' .json >> '{{home}}/seats'; case '{{output}}' in *parity-reviewer*) printf '{parity}' ;; *) printf '{APPROVE}' ;; esac > '{{output}}'"
            ),
        ];
        runner.output = OutputMode::File;
    }
}

fn run_review(
    agents: &Agents,
    task: &Task,
    pinned: Option<&ReviewerPin>,
) -> (Result<Step, GateError>, String) {
    let folder = tempfile::tempdir().unwrap();
    worktree(folder.path());
    git(folder.path(), &["update-ref", "refs/heads/main", "HEAD"]).unwrap();
    let development = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    fs::copy(
        development.join("rules.yaml"),
        folder.path().join(".development/rules.yaml"),
    )
    .unwrap();
    fs::copy(
        development.join("schema/review.schema.json"),
        folder.path().join(".development/schema/review.schema.json"),
    )
    .unwrap();
    let brief = brief(task);
    let places = places(folder.path());
    let attempt = Attempt {
        agents,
        brief: &brief,
        places: &places,
        number: 1,
        earlier: None,
        reviewer: pinned,
        resume: None,
    };
    let worker = find_runner(agents, "claude-worker").unwrap();
    let step = review(&attempt, worker, folder.path());
    let seats = fs::read_to_string(folder.path().join("seats")).unwrap();
    (step, seats)
}

#[test]
fn a_standard_approval_of_a_task_that_cites_litellm_gets_a_parity_pass() {
    let (mut agents, task) = fixtures();
    review_answers(&mut agents, APPROVE);
    let (step, seats) = run_review(&agents, &task, None);
    assert_eq!(seats, "attempt-1-reviewer\nattempt-1-parity-reviewer\n");
    let Ok(Step::Reviewed(round)) = step else {
        panic!("the review should end in a round");
    };
    assert_eq!(round.verdict, ReviewVerdict::Approve);
    assert_eq!(round.reviewer.panel, Panel::Parity);
}

#[test]
fn a_task_without_references_gets_one_review() {
    let (mut agents, mut task) = fixtures();
    task.references.clear();
    review_answers(&mut agents, APPROVE);
    let (step, seats) = run_review(&agents, &task, None);
    assert_eq!(seats, "attempt-1-reviewer\n");
    let Ok(Step::Reviewed(round)) = step else {
        panic!("the review should end in a round");
    };
    assert_eq!(round.reviewer.panel, Panel::Standard);
}

#[test]
fn a_round_after_the_parity_pass_goes_to_the_parity_reviewer() {
    let (mut agents, task) = fixtures();
    review_answers(&mut agents, APPROVE);
    let pinned = ReviewerPin {
        runner: "claude-reviewer".to_owned(),
        model: "claude-opus-5-5".to_owned(),
        panel: Panel::Parity,
    };
    let (step, seats) = run_review(&agents, &task, Some(&pinned));
    assert_eq!(seats, "attempt-1-parity-reviewer\n");
    let Ok(Step::Reviewed(round)) = step else {
        panic!("the review should end in a round");
    };
    assert_eq!(round.reviewer, pinned);
}

#[test]
fn a_parity_finding_under_another_rule_goes_back_once_then_fails() {
    let (mut agents, task) = fixtures();
    let style = r#"{"task":"B000","verdict":"changes","findings":[{"rule":"clear-names","origin":"ours","location":".development/rules.yaml:1","problem":"a vague name","fix":"rename it"}]}"#;
    review_answers(&mut agents, style);
    let (step, seats) = run_review(&agents, &task, None);
    assert_eq!(
        seats,
        "attempt-1-reviewer\nattempt-1-parity-reviewer\nattempt-1-parity-reviewer-again\n"
    );
    let Err(error) = step else {
        panic!("the review should fail");
    };
    assert!(
        error
            .to_string()
            .contains("cites clear-names, and the parity review reports only")
    );
}
