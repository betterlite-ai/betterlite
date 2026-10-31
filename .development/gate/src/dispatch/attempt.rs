use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::answer::parse_answer;
use crate::checks;
use crate::checks::Selection;
use crate::dependencies::added_dependencies;
use crate::development::Development;
use crate::deviations;
use crate::dispatch::seat::{Run, Seat, invoke};
use crate::dispatch::{Attempt, Step, prompt, review};
use crate::error::GateError;
use crate::facts::resolve_base;
use crate::gate_lock;
use crate::git::git;
use crate::report_check::report_problems;
use crate::spec::agents::{Rung, Runner};
use crate::spec::report::{Report, ReportStatus};

fn clear_leftovers(journal: &Path) -> Result<(), GateError> {
    // a hook whose worker hit its timeout keeps running the checks and would race this attempt's own
    gate_lock::stop_orphan(&journal.join("gate"), gate_lock::GRACE)?;
    let blocks = journal.join("hook-blocks");
    if blocks.exists() {
        fs::remove_file(&blocks).map_err(|source| GateError::Write {
            path: blocks.clone(),
            source,
        })?;
    }
    Ok(())
}

fn worker_report(
    attempt: &Attempt<'_>,
    runner: &Runner,
    rung: &Rung,
    feedback: &[String],
) -> Result<Result<Report, Option<Step>>, GateError> {
    clear_leftovers(attempt.places.journal)?;
    let prompt = prompt::worker(attempt.brief, feedback);
    let run = invoke(attempt, runner, rung, Seat::Worker, &prompt)?;
    gate_lock::stop_orphan(&attempt.places.journal.join("gate"), gate_lock::GRACE)?;
    let text = match run {
        Run::Answer(text) => text,
        Run::Silent => {
            return Ok(Err(Some(Step::Retry(vec![
                "The previous run ended without a final answer.".to_owned(),
            ]))));
        }
        Run::TimedOut => {
            return Ok(Err(Some(Step::Retry(vec![format!(
                "The previous run hit its {} minute timeout before its final answer; its work stays in the worktree.",
                rung.timeout_minutes
            )]))));
        }
        Run::Limited => return Ok(Err(None)),
    };
    Ok(match parse_answer::<Report>(&text, "the report") {
        Ok(report) if report.status == ReportStatus::Blocked => {
            Err(Some(Step::Climb(vec![format!(
                "A previous worker reported blocked: {}",
                report.blocker
            )])))
        }
        Ok(report) => Ok(report),
        Err(error) => Err(Some(Step::Fix(vec![format!(
            "The final answer did not match the report format: {error}"
        )]))),
    })
}

fn failures(
    worktree: &Development,
    task_id: &str,
    report: &Report,
) -> Result<Vec<String>, GateError> {
    let task = worktree.task(task_id)?;
    let verdict = checks::run(worktree, &task, &Selection::default())?;
    let added = added_dependencies(&worktree.root, &resolve_base(&worktree.root)?)?;
    let mut problems = report_problems(report, &task, &added);
    let log = worktree
        .root
        .join(format!("docs/deviations/{task_id}.yaml"));
    if log.exists() {
        problems.extend(deviations::file_problems(&log, &worktree.backlog));
    }
    if verdict.passed && problems.is_empty() {
        return Ok(Vec::new());
    }
    let failing = verdict
        .lines
        .into_iter()
        .filter(|line| !line.starts_with("PASS") && !line.starts_with("SKIP"));
    Ok(failing.chain(problems).collect())
}

fn worker_edits(worktree: &Path, start: &str) -> Result<Vec<String>, GateError> {
    let committed = git(
        worktree,
        &["diff", "--name-only", start, "HEAD", "--", ".development"],
    )?;
    let pending = git(worktree, &["status", "--porcelain", "--", ".development"])?;
    // a file that matches origin/main came in with a merge of main, not from the worker
    let changed_from_main = git(
        worktree,
        &["diff", "--name-only", "origin/main", "--", ".development"],
    )?;
    let differs: BTreeSet<&str> = changed_from_main.lines().collect();
    let pending_paths = pending.lines().filter_map(|line| {
        let path = line.get(3..)?;
        (line.starts_with("??") || differs.contains(path)).then_some(path)
    });
    Ok(committed
        .lines()
        .filter(|path| differs.contains(path))
        .chain(pending_paths)
        .map(|path| {
            format!(
                "The worker changed {path}; only the architect edits .development/, so revert it."
            )
        })
        .collect())
}

pub(crate) fn remove_extra_targets(worktree: &Path) -> Result<Vec<String>, GateError> {
    let target = worktree.join("target");
    if !target.is_dir() {
        return Ok(Vec::new());
    }
    let entries = fs::read_dir(&target).map_err(|source| GateError::Read {
        path: target.clone(),
        source,
    })?;
    let mut notes = Vec::new();
    for found in entries {
        let entry = found.map_err(|source| GateError::Read {
            path: target.clone(),
            source,
        })?;
        let path = entry.path();
        // the coverage check keeps its own target between runs
        if entry.file_name() == "llvm-cov-target" || !path.join("CACHEDIR.TAG").exists() {
            continue;
        }
        fs::remove_dir_all(&path).map_err(|source| GateError::Write {
            path: path.clone(),
            source,
        })?;
        notes.push(format!(
            "The gate deleted target/{}, a second cargo target: never set CARGO_TARGET_DIR or --target-dir, since it builds the whole workspace again from scratch.",
            entry.file_name().display()
        ));
    }
    Ok(notes)
}

pub(crate) fn attempt_once(
    attempt: &Attempt<'_>,
    runner: &Runner,
    rung: &Rung,
    feedback: &[String],
) -> Result<Option<Step>, GateError> {
    let start = git(attempt.places.worktree, &["rev-parse", "HEAD"])?;
    let report = match worker_report(attempt, runner, rung, feedback)? {
        Ok(report) => report,
        Err(step) => return Ok(step),
    };
    let touched = worker_edits(attempt.places.worktree, start.trim())?;
    if !touched.is_empty() {
        return Ok(Some(Step::Fix(touched)));
    }
    let worktree = Development::open(attempt.places.worktree)?;
    let failing = failures(&worktree, &attempt.brief.task.id, &report)?;
    if !failing.is_empty() {
        return Ok(Some(Step::Fix(failing)));
    }
    review::review(attempt, runner, &worktree).map(Some)
}
