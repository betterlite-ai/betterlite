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

// the dispatcher's final gate runs only these, since none builds or runs a test; every other check runs in CI on the task's pull request
pub(crate) fn lint(id: &str) -> bool {
    match id {
        "clippy" | "deny" | "shear" | "helper-python" => true,
        _ => prerequisite(id),
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
    })
}

pub(super) fn schedule<'spec>(
    development: &'spec Development,
    task: &'spec Task,
    selection: &Selection,
) -> Result<[Vec<&'spec Command>; 3], GateError> {
    let (later, first): (Vec<&Command>, Vec<&Command>) = commands(development, task)?
        .into_iter()
        .filter(|check| {
            (selection.only.is_empty() || selection.only.contains(&check.id))
                && !selection.skip.contains(&check.id)
        })
        .partition(|check| check.after_green);
    let (prerequisites, expensive) = first
        .into_iter()
        .partition(|command| prerequisite(&command.id));
    Ok([prerequisites, expensive, later])
}
