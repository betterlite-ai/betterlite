use crate::checks::{Selection, commands};
use crate::development::Development;
use crate::error::GateError;
use crate::spec::command::Command;
use crate::spec::task::Task;

pub(super) fn validate(
    development: &Development,
    task: &Task,
    selection: &Selection,
) -> Result<(), GateError> {
    let selected = commands(development, task)?;
    let available: Vec<&str> = development
        .catalog
        .base
        .builtin
        .iter()
        .map(|check| <&str>::from(check.id))
        .chain(selected.iter().map(|check| check.id.as_str()))
        .collect();
    for id in &selection.only {
        if !available.contains(&id.as_str()) {
            return Err(GateError::Selection(format!(
                "{id} is not a check of {}",
                task.id
            )));
        }
        if selection.skip.contains(id) {
            return Err(GateError::Selection(format!(
                "{id} is both selected and skipped"
            )));
        }
    }
    for id in &selection.skip {
        if !available.contains(&id.as_str())
            && !development
                .catalog
                .optional
                .iter()
                .any(|check| check.id == *id)
        {
            return Err(GateError::Selection(format!("unknown skipped check {id}")));
        }
    }
    if !available.iter().any(|id| {
        (selection.only.is_empty() || selection.only.iter().any(|only| only == id))
            && !selection.skip.iter().any(|skip| skip == id)
    }) {
        return Err(GateError::Selection("no checks selected".to_owned()));
    }
    validate_beside(&selected, task, &selection.beside)
}

fn validate_beside(selected: &[&Command], task: &Task, beside: &[String]) -> Result<(), GateError> {
    for id in beside {
        let check = selected
            .iter()
            .find(|candidate| candidate.id == *id)
            .ok_or_else(|| {
                GateError::Selection(format!("{id} is not a command check of {}", task.id))
            })?;
        if check.after_green {
            return Err(GateError::Selection(format!(
                "{id} runs once every other check passed, so it cannot run beside them"
            )));
        }
    }
    Ok(())
}

pub(crate) fn prerequisite(id: &str) -> bool {
    [
        "committed",
        "whitespace",
        "specs-untouched",
        "fmt",
        "hooks",
        "similarity",
        "notice",
        "no-enterprise",
        "no-attribution",
        "workflows",
    ]
    .contains(&id)
}

// cargo gate list --local prints these, and CI's lint part runs them beside clippy
pub(crate) fn lint(id: &str) -> bool {
    match id {
        "deny" | "shear" => true,
        _ => prerequisite(id),
    }
}

// stage 3: CI runs every other check on the task's pull request, and on 11-13 the local hooks, mypy and basedpyright took 1 to 3 minutes a call under load
pub(crate) fn committed() -> Selection {
    Selection {
        only: vec!["committed".to_owned()],
        skip: Vec::new(),
        beside: Vec::new(),
    }
}

pub(crate) fn builtins_and(
    development: &Development,
    task: &Task,
    keep: fn(&str) -> bool,
) -> Result<Selection, GateError> {
    let only = development
        .catalog
        .base
        .builtin
        .iter()
        .map(|check| <&str>::from(check.id).to_owned())
        .chain(
            commands(development, task)?
                .iter()
                .filter(|check| keep(&check.id))
                .map(|check| check.id.clone()),
        )
        .collect();
    Ok(Selection {
        only,
        skip: Vec::new(),
        beside: Vec::new(),
    })
}

pub(super) fn schedule<'spec>(
    development: &'spec Development,
    task: &'spec Task,
    selection: &Selection,
) -> Result<[Vec<&'spec Command>; 4], GateError> {
    let (later, first): (Vec<&Command>, Vec<&Command>) = commands(development, task)?
        .into_iter()
        .filter(|check| {
            (selection.only.is_empty() || selection.only.contains(&check.id))
                && !selection.skip.contains(&check.id)
        })
        .partition(|check| check.after_green);
    let (beside, in_order): (Vec<&Command>, Vec<&Command>) = first
        .into_iter()
        .partition(|check| selection.beside.contains(&check.id));
    let (prerequisites, expensive) = in_order
        .into_iter()
        .partition(|command| prerequisite(&command.id));
    Ok([prerequisites, expensive, later, beside])
}
