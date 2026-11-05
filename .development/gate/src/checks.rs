mod execute;
pub(crate) mod history;
pub(crate) mod selection;
mod tests;

use std::fs;
use std::io;
use std::path::Path;
use std::time::Instant;

use crate::builtins::{Context, run_builtin};
use crate::development::Development;
use crate::error::GateError;
use crate::facts::Facts;
use crate::gate_lock;
use crate::journal::journal_dir;
use crate::spec::command::Command;
use crate::spec::task::Task;

pub(crate) struct Verdict {
    pub(crate) lines: Vec<String>,
    pub(crate) passed: bool,
    pub(crate) failed: Vec<String>,
}

#[derive(Default)]
pub(crate) struct Selection {
    pub(crate) only: Vec<String>,
    pub(crate) skip: Vec<String>,
}

pub(crate) enum Outcome {
    Pass,
    Skip(String),
    Fail { why: String, hints: Vec<String> },
}

pub(crate) fn commands<'spec>(
    development: &'spec Development,
    task: &'spec Task,
) -> Result<Vec<&'spec Command>, GateError> {
    let mut selected: Vec<&Command> = development.catalog.base.commands.iter().collect();
    let mut optional_ids: Vec<&str> = task.checks.optional.iter().map(String::as_str).collect();
    if !task.oracle_cases.is_empty() && !optional_ids.contains(&"oracle") {
        optional_ids.push("oracle");
    }
    for id in optional_ids {
        let check = development
            .catalog
            .optional
            .iter()
            .find(|candidate| candidate.id == id)
            .ok_or_else(|| GateError::UnknownCheck(id.to_owned()))?;
        selected.push(check);
    }
    selected.extend(&task.checks.commands);
    Ok(selected)
}

fn builtin_outcomes(
    development: &Development,
    task: &Task,
    facts: &Facts,
    selection: &Selection,
) -> Vec<(String, u64, Outcome)> {
    let context = Context {
        development,
        task,
        facts,
    };
    let mut outcomes = Vec::new();
    for builtin in &development.catalog.base.builtin {
        let id = <&str>::from(builtin.id);
        if (!selection.only.is_empty() && !selection.only.iter().any(|selected| selected == id))
            || selection.skip.iter().any(|skipped| skipped == id)
        {
            continue;
        }
        let started = Instant::now();
        let outcome = match run_builtin(builtin.id, &context) {
            Ok(problems) if problems.is_empty() => Outcome::Pass,
            Ok(problems) => Outcome::Fail {
                why: builtin.why.clone(),
                hints: problems,
            },
            Err(error) => Outcome::Fail {
                why: builtin.why.clone(),
                hints: vec![error.to_string()],
            },
        };
        outcomes.push((id.to_owned(), started.elapsed().as_secs(), outcome));
    }
    outcomes
}

pub(crate) fn run(
    development: &Development,
    task: &Task,
    selection: &Selection,
) -> Result<Verdict, GateError> {
    selection::validate(development, task, selection)?;
    let root = &development.root;
    let facts = Facts::collect(root)?;
    let log_dir = journal_dir(root, &task.id)?.join("gate");
    fs::create_dir_all(&log_dir).map_err(|source| GateError::Write {
        path: log_dir.clone(),
        source,
    })?;
    // a dispatch reads the holder's process group from it to stop a check run whose worker died
    let _held = gate_lock::hold(&log_dir, &mut io::stderr())?;
    let run_dir = history::start(&log_dir)?;
    let verdict = run_checks(development, task, selection, &facts, &run_dir);
    if let Err(error) = &verdict {
        // a dispatch takes a run with neither result.json nor error.txt for one cut off mid-way
        let path = run_dir.join("error.txt");
        fs::write(&path, error.to_string()).map_err(|source| GateError::Write { path, source })?;
    }
    verdict
}

fn run_checks(
    development: &Development,
    task: &Task,
    selection: &Selection,
    facts: &Facts,
    log_dir: &Path,
) -> Result<Verdict, GateError> {
    let root = &development.root;
    let mut outcomes = builtin_outcomes(development, task, facts, selection);
    let environment = [
        ("GATE_TASK", task.id.clone()),
        ("GATE_BASE", facts.base.clone()),
        ("GATE_LOG_DIR", log_dir.display().to_string()),
        ("GATE_ORACLE_CASES", task.oracle_cases.join(" ")),
    ];
    let scheduled = selection::schedule(development, task, selection)?;
    let reasons = [
        None,
        Some("fix the failed prerequisites before expensive checks"),
        Some("runs once every other check passes"),
    ];
    for (group, reason) in scheduled.into_iter().zip(reasons) {
        let blocked = reason.filter(|_| {
            outcomes
                .iter()
                .any(|(_, _, outcome)| matches!(outcome, Outcome::Fail { .. }))
        });
        for command in group {
            let started = Instant::now();
            let outcome = match blocked {
                Some(message) => Outcome::Skip(message.to_owned()),
                None => run_command(root, log_dir, command, &environment)?,
            };
            outcomes.push((command.id.clone(), started.elapsed().as_secs(), outcome));
        }
    }
    history::save(root, log_dir, &facts.base, selection, &outcomes)?;
    Ok(render(task, &facts.base, log_dir, outcomes))
}

fn run_command(
    root: &Path,
    log_dir: &Path,
    command: &Command,
    environment: &[(&str, String)],
) -> Result<Outcome, GateError> {
    match &command.when_exists {
        Some(path) if !root.join(path).exists() => {
            Ok(Outcome::Skip(format!("{path} does not exist")))
        }
        Some(_) | None => execute::execute(root, log_dir, command, environment),
    }
}

fn render(
    task: &Task,
    base: &str,
    log_dir: &Path,
    outcomes: Vec<(String, u64, Outcome)>,
) -> Verdict {
    let short_base: String = base.chars().take(12).collect();
    let mut lines = vec![format!(
        "gate {} against {short_base}: {} checks, logs in {}",
        task.id,
        outcomes.len(),
        log_dir.display()
    )];
    let mut passed = !outcomes.is_empty();
    let mut failed = Vec::new();
    if outcomes.is_empty() {
        lines.push("FAIL selection: no checks selected".to_owned());
        failed.push("selection".to_owned());
    }
    for (id, seconds, outcome) in outcomes {
        match outcome {
            Outcome::Pass => lines.push(format!("PASS {id} ({seconds}s)")),
            Outcome::Skip(reason) => lines.push(format!("SKIP {id}: {reason}")),
            Outcome::Fail { why, hints } => {
                passed = false;
                lines.push(format!("FAIL {id} ({seconds}s): {why}"));
                failed.push(id);
                lines.extend(
                    hints
                        .iter()
                        .map(|hint| format!("     {}", hint.chars().take(240).collect::<String>())),
                );
            }
        }
    }
    lines.push(if passed { "gate passed" } else { "gate failed" }.to_owned());
    Verdict {
        lines,
        passed,
        failed,
    }
}
