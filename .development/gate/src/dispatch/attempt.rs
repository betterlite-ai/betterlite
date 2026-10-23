use std::fs;

use crate::answer::parse_answer;
use crate::checks;
use crate::dependencies::added_dependencies;
use crate::development::Development;
use crate::dispatch::seat::{Seat, invoke};
use crate::dispatch::{Attempt, Step, prompt, review};
use crate::error::GateError;
use crate::facts::resolve_base;
use crate::report_check::report_problems;
use crate::spec::agents::{Rung, Runner};
use crate::spec::report::{Report, ReportStatus};

fn worker_report(
    attempt: &Attempt<'_>,
    runner: &Runner,
    rung: &Rung,
    feedback: &[String],
) -> Result<Result<Report, Step>, GateError> {
    let blocks = attempt.journal.join("hook-blocks");
    if blocks.exists() {
        fs::remove_file(&blocks).map_err(|source| GateError::Write {
            path: blocks.clone(),
            source,
        })?;
    }
    let Some(text) = invoke(
        attempt,
        runner,
        rung,
        Seat::Worker,
        &prompt::worker(attempt.brief, feedback),
    )?
    else {
        return Ok(Err(Step::Retry(vec![
            "The previous run ended without a final answer.".to_owned(),
        ])));
    };
    Ok(match parse_answer::<Report>(&text, "the report") {
        Ok(report) if report.status == ReportStatus::Blocked => Err(Step::Climb(vec![format!(
            "A previous worker reported blocked: {}",
            report.blocker
        )])),
        Ok(report) => Ok(report),
        Err(error) => Err(Step::Retry(vec![format!(
            "The final answer did not match the report format: {error}"
        )])),
    })
}

fn failures(
    worktree: &Development,
    task_id: &str,
    report: &Report,
) -> Result<Vec<String>, GateError> {
    let verdict = checks::run(worktree, &worktree.task(task_id)?, &[])?;
    let added = added_dependencies(&worktree.root, &resolve_base(&worktree.root)?)?;
    let problems = report_problems(report, task_id, &added);
    if verdict.passed && problems.is_empty() {
        return Ok(Vec::new());
    }
    let failing = verdict
        .lines
        .into_iter()
        .filter(|line| !line.starts_with("PASS") && !line.starts_with("SKIP"));
    Ok(failing.chain(problems).collect())
}

pub(crate) fn attempt_once(
    attempt: &Attempt<'_>,
    runner: &Runner,
    rung: &Rung,
    feedback: &[String],
) -> Result<Step, GateError> {
    let report = match worker_report(attempt, runner, rung, feedback)? {
        Ok(report) => report,
        Err(step) => return Ok(step),
    };
    let worktree = Development::open(attempt.worktree)?;
    let failing = failures(&worktree, &attempt.brief.task.id, &report)?;
    if !failing.is_empty() {
        return Ok(Step::Retry(failing));
    }
    review::review(attempt, runner, &worktree)
}
