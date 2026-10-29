mod execute;
mod tests;

use std::fs;
use std::path::Path;
use std::time::Instant;

use crate::builtins::{Context, run_builtin};
use crate::development::Development;
use crate::error::GateError;
use crate::facts::Facts;
use crate::journal::journal_dir;
use crate::spec::command::Command;
use crate::spec::task::Task;

pub(crate) struct Verdict {
    pub(crate) lines: Vec<String>,
    pub(crate) passed: bool,
}

#[derive(Default)]
pub(crate) struct Selection {
    pub(crate) only: Vec<String>,
    pub(crate) skip: Vec<String>,
}

impl Selection {
    fn includes(&self, id: &str) -> bool {
        (self.only.is_empty() || self.only.iter().any(|selected| selected == id))
            && !self.skip.iter().any(|skipped| skipped == id)
    }
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
        let id: &str = builtin.id.into();
        if !selection.includes(id) {
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
    let root = &development.root;
    let facts = Facts::collect(root)?;
    let log_dir = journal_dir(root, &task.id)?.join("gate");
    fs::create_dir_all(&log_dir).map_err(|source| GateError::Write {
        path: log_dir.clone(),
        source,
    })?;
    let mut outcomes = builtin_outcomes(development, task, &facts, selection);
    let environment = [
        ("GATE_TASK", task.id.clone()),
        ("GATE_BASE", facts.base.clone()),
        ("GATE_LOG_DIR", log_dir.display().to_string()),
        ("GATE_ORACLE_CASES", task.oracle_cases.join(" ")),
    ];
    let (later, first): (Vec<&Command>, Vec<&Command>) = commands(development, task)?
        .into_iter()
        .filter(|check| selection.includes(&check.id))
        .partition(|check| check.after_green);
    for command in first {
        let started = Instant::now();
        let outcome = run_command(root, &log_dir, command, &environment)?;
        outcomes.push((command.id.clone(), started.elapsed().as_secs(), outcome));
    }
    let red = outcomes
        .iter()
        .any(|(_, _, outcome)| matches!(outcome, Outcome::Fail { .. }));
    for command in later {
        let started = Instant::now();
        let outcome = if red {
            Outcome::Skip("runs once every other check passes".to_owned())
        } else {
            run_command(root, &log_dir, command, &environment)?
        };
        outcomes.push((command.id.clone(), started.elapsed().as_secs(), outcome));
    }
    Ok(render(task, &facts.base, &log_dir, outcomes))
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
    let mut passed = true;
    for (id, seconds, outcome) in outcomes {
        match outcome {
            Outcome::Pass => lines.push(format!("PASS {id} ({seconds}s)")),
            Outcome::Skip(reason) => lines.push(format!("SKIP {id}: {reason}")),
            Outcome::Fail { why, hints } => {
                passed = false;
                lines.push(format!("FAIL {id} ({seconds}s): {why}"));
                lines.extend(
                    hints
                        .iter()
                        .map(|hint| format!("     {}", hint.chars().take(240).collect::<String>())),
                );
            }
        }
    }
    lines.push(if passed { "gate passed" } else { "gate failed" }.to_owned());
    Verdict { lines, passed }
}
