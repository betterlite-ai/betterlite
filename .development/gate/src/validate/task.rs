use std::collections::BTreeSet;

use crate::builtins::globs::glob_set;
use crate::ids::check_id_problem;
use crate::spec::backlog::BacklogEntry;
use crate::spec::catalog::Catalog;
use crate::spec::task::Task;
use crate::validate::{catalog, duplicates};

fn text_problems(task: &Task) -> Vec<String> {
    let id = &task.id;
    let mut problems = Vec::new();
    if task.title.trim().is_empty() || task.description.trim().is_empty() {
        problems.push(format!("{id}: title and description must not be empty"));
    }
    if task.scope.paths.is_empty() || task.dod.is_empty() || task.budget.max_changed_lines == 0 {
        problems.push(format!(
            "{id}: scope.paths, dod and budget.max_changed_lines must not be empty or zero"
        ));
    }
    if let Err(error) = glob_set(&task.scope.paths) {
        problems.push(format!("{id}: {error}"));
    }
    let dod_texts = task.dod.iter().map(|item| &item.text);
    let mut notes = task
        .out_of_scope
        .iter()
        .chain(&task.pitfalls)
        .chain(&task.oracle_cases)
        .chain(dod_texts);
    if notes.any(|note| note.trim().is_empty()) {
        problems.push(format!(
            "{id}: dod texts, out_of_scope, pitfalls and oracle_cases hold no empty strings"
        ));
    }
    problems
}

fn link_problems(task: &Task) -> Vec<String> {
    let id = &task.id;
    let mut problems: Vec<String> = task
        .references
        .iter()
        .filter(|reference| {
            !reference.url.starts_with("https://") || reference.why.trim().is_empty()
        })
        .map(|reference| {
            format!(
                "{id}: reference {} needs an https url and a why",
                reference.url
            )
        })
        .collect();
    for deviation in &task.deviations {
        let link_ok = deviation
            .link
            .as_ref()
            .is_none_or(|link| link.starts_with("https://"));
        if deviation.what.trim().is_empty() || deviation.why.trim().is_empty() || !link_ok {
            problems.push(format!(
                "{id}: a deviation needs what, why and an https link when it has one"
            ));
        }
    }
    problems
}

fn available_checks<'spec>(
    catalog: &'spec Catalog,
    task: &'spec Task,
    problems: &mut Vec<String>,
) -> BTreeSet<&'spec str> {
    let mut available: BTreeSet<&str> = catalog
        .base
        .builtin
        .iter()
        .map(|check| <&str>::from(check.id))
        .collect();
    available.extend(catalog.base.commands.iter().map(|check| check.id.as_str()));
    for optional in &task.checks.optional {
        if catalog.optional.iter().any(|check| &check.id == optional) {
            available.insert(optional);
        } else {
            problems.push(format!(
                "{}: {optional} is not an optional check in checks.yaml",
                task.id
            ));
        }
    }
    if !task.oracle_cases.is_empty() {
        available.insert("oracle");
    }
    available.extend(task.checks.commands.iter().map(|check| check.id.as_str()));
    available
}

fn proof_problems(catalog: &Catalog, task: &Task) -> Vec<String> {
    let id = &task.id;
    let mut problems = Vec::new();
    let available = available_checks(catalog, task, &mut problems);
    let catalog_ids = catalog::ids(catalog);
    let own: Vec<&str> = task
        .checks
        .commands
        .iter()
        .map(|check| check.id.as_str())
        .collect();
    problems.extend(
        duplicates(&own)
            .into_iter()
            .map(|check| format!("{id}: command {check} is defined twice")),
    );
    let misnamed = own
        .iter()
        .filter(|check| check_id_problem(check).is_some() || catalog_ids.contains(check));
    problems.extend(misnamed.map(|check| {
        format!("{id}: command id {check} must be kebab-case and not reuse a checks.yaml id")
    }));
    let unknown = task
        .dod
        .iter()
        .flat_map(|item| &item.proof)
        .filter(|proof| !available.contains(proof.as_str()));
    problems.extend(
        unknown.map(|proof| format!("{id}: DoD proof {proof} is not a check this task runs")),
    );
    problems
}

pub(crate) fn problems(catalog: &Catalog, entry: &BacklogEntry, task: &Task) -> Vec<String> {
    let mut problems = Vec::new();
    if task.id != entry.id {
        problems.push(format!("{}: the file says id {}", entry.id, task.id));
    }
    problems.extend(text_problems(task));
    problems.extend(link_problems(task));
    problems.extend(proof_problems(catalog, task));
    problems
}
