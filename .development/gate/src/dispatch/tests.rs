#![cfg(test)]

use std::fs::{self, File};
use std::ops::ControlFlow;
use std::path::Path;

use crate::dispatch::attempt::remove_extra_targets;
use crate::dispatch::progress::{NextRun, Progress, ReviewerPin, WorkerSession};
use crate::dispatch::prompt::Brief;
use crate::dispatch::review::reviewer_rungs;
use crate::dispatch::vendor::mark;
use crate::dispatch::{
    EarlierReview, Places, Round, Step, apply_step, climb_rung, find_runner, note_blocked,
    overflow_rung, pick_rung, run_ladder,
};
use crate::error::GateError;
use crate::git::git;
use crate::spec::agents::{Agents, OutputMode};
use crate::spec::review::ReviewVerdict;
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
        next: NextRun::Fresh,
        fix_turns: 0,
        granted: 0,
        reviewer: None,
        session: None,
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
    saved.rung_used = 1;
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
        model: "sonnet".to_owned(),
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
    let first = &agents.roles.worker.ladder[0];
    let lines = climb_rung(&agents, &brief, &places, first, &mut progress).unwrap();
    assert_eq!(lines, None);
    assert!(progress.limited);
    assert_eq!((progress.number, progress.used), (0, 0));
}

#[test]
fn a_limited_rung_lends_its_try_to_a_later_rung_and_keeps_its_attempts() {
    let (agents, _) = fixtures();
    let folder = tempfile::tempdir().unwrap();
    let places = Places {
        worktree: folder.path(),
        journal: folder.path(),
        git_common_dir: folder.path(),
        home: folder.path(),
        writable: &[],
    };
    let ladder = &agents.roles.worker.ladder;
    let own = pick_rung(&agents, &places, 0, &ladder[0]).unwrap().unwrap();
    assert_eq!(
        (own.rung.model.as_str(), own.counted),
        ("gpt-reserve", true)
    );
    mark(
        folder.path(),
        "openai",
        "ERROR: You've hit your usage limit.",
    )
    .unwrap();
    let borrowed = pick_rung(&agents, &places, 0, &ladder[0]).unwrap().unwrap();
    assert_eq!(
        (
            borrowed.runner.id.as_str(),
            borrowed.rung.model.as_str(),
            borrowed.counted
        ),
        ("claude-worker", "sonnet", false)
    );
    let last = ladder.len() - 1;
    mark(folder.path(), "anthropic", "You've hit your limit").unwrap();
    assert!(
        pick_rung(&agents, &places, last, &ladder[last])
            .unwrap()
            .is_none()
    );
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
    saved.rung_used = 1;
    saved.save(folder.path()).unwrap();
    let mut loaded = Progress::load(folder.path(), Vec::new()).unwrap();
    let error = run_ladder(&agents, &brief, &places, &mut loaded, 0).unwrap_err();
    assert!(matches!(error, GateError::UsageLimit { .. }));
    assert!(error.to_string().contains(
        "every runner left on the worker ladder is at its usage limit; last feedback: the saved findings"
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
        reviewer_rungs(&agents, level, runner, None)
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

#[test]
fn the_reviewer_of_the_last_round_comes_first() {
    let (agents, _) = fixtures();
    let codex = find_runner(&agents, "codex-worker").unwrap();
    let order = |runner: &str, model: &str| {
        let pin = ReviewerPin {
            runner: runner.to_owned(),
            model: model.to_owned(),
        };
        reviewer_rungs(&agents, ReviewLevel::Standard, codex, Some(&pin))
            .into_iter()
            .map(|rung| rung.model.clone())
            .collect::<Vec<String>>()
    };
    assert_eq!(
        order("codex-reviewer", "gpt-6-astra"),
        ["gpt-6-astra", "sonnet"]
    );
    assert_eq!(
        order("claude-reviewer", "sonnet"),
        ["sonnet", "gpt-6-astra"]
    );
    // a pin whose rung left agents.yaml falls back to the usual order
    assert_eq!(
        order("codex-reviewer", "gpt-retired"),
        ["sonnet", "gpt-6-astra"]
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
    let anthropic = hold_seats(folder.path(), "anthropic", 12);
    assert!(overflow_rung(&agents, &places, 0, codex).unwrap().is_none());
    drop(anthropic);
    mark(folder.path(), "anthropic", "You've hit your limit").unwrap();
    assert!(overflow_rung(&agents, &places, 0, codex).unwrap().is_none());
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
        model: "sonnet".to_owned(),
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
        let step = Step::Fix(vec!["a red check".to_owned()]);
        let flow = apply_step(
            &agents,
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
    let step = Step::Retry(vec!["no answer".to_owned()]);
    let flow = apply_step(
        &agents,
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
        &agents,
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
        &agents,
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
        &agents,
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
        ControlFlow::Break(Some(vec![
            ready,
            "an earlier LiteLLM bug".to_owned(),
            "a LiteLLM bug".to_owned(),
        ]))
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
    progress.grant(ladder, 0);
    assert_eq!((progress.rung, progress.granted), (ladder.len(), 0));
    progress.grant(ladder, 2);
    assert_eq!(
        (progress.rung, progress.rung_used, progress.granted),
        (last, ladder[last].attempts, 2)
    );
    progress.rung = ladder.len();
    progress.grant(ladder, 1);
    assert_eq!(
        (progress.rung, progress.rung_used, progress.granted),
        (last, ladder[last].attempts + 2, 3)
    );
    let mut climbing = self::progress();
    climbing.rung = 1;
    climbing.rung_used = 1;
    climbing.grant(ladder, 1);
    assert_eq!(
        (climbing.rung, climbing.rung_used, climbing.granted),
        (1, 1, 1)
    );
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
    let first = &agents.roles.worker.ladder[0];
    let lines = climb_rung(
        &agents,
        &brief(&task),
        &places(folder.path()),
        first,
        &mut progress,
    )
    .unwrap();
    assert_eq!(lines, None);
    let runs = first
        .attempts
        .saturating_mul(agents.policy.max_fix_turns.saturating_add(1));
    assert_eq!(
        (progress.number, progress.used, progress.rung_used),
        (runs, first.attempts, first.attempts)
    );
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
    progress.rung_used = ladder[last].attempts;
    progress.grant(ladder, 1);
    for _ in 0..2_u32 {
        let lines = climb_rung(
            &agents,
            &brief(&task),
            &places(folder.path()),
            &ladder[last],
            &mut progress,
        )
        .unwrap();
        assert_eq!(lines, None);
    }
    assert_eq!(
        (progress.number, progress.used, progress.rung_used),
        (
            agents.policy.max_fix_turns + 1,
            agents.policy.max_worker_attempts + 1,
            ladder[last].attempts + 1
        )
    );
}

fn claude_answers(agents: &mut Agents) {
    let answer = r#"printf '{"structured_output":{},"session_id":"s1"}'"#;
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
    claude_answers(&mut agents);
    let folder = tempfile::tempdir().unwrap();
    worktree(folder.path());
    let first = &agents.roles.worker.ladder[0];
    let mut progress = progress();
    climb_rung(
        &agents,
        &brief(&task),
        &places(folder.path()),
        first,
        &mut progress,
    )
    .unwrap();
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
    claude_answers(&mut agents);
    let folder = tempfile::tempdir().unwrap();
    worktree(folder.path());
    let first = &agents.roles.worker.ladder[0];
    let mut progress = progress();
    progress.next = NextRun::Fix;
    progress.feedback = vec!["a red check".to_owned()];
    progress.session = Some(WorkerSession {
        runner: first.runner.clone(),
        model: "another".to_owned(),
        id: "s0".to_owned(),
    });
    climb_rung(
        &agents,
        &brief(&task),
        &places(folder.path()),
        first,
        &mut progress,
    )
    .unwrap();
    let resumed = fs::read_to_string(folder.path().join("resumed")).unwrap();
    assert!(!resumed.contains("s0"), "{resumed}");
}
