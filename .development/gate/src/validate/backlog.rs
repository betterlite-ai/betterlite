use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use crate::error::GateError;
use crate::ids::task_id_problem;
use crate::spec::backlog::{Backlog, BacklogEntry, Status};
use crate::validate::duplicates;

fn entry_problems(root: &Path, entry: &BacklogEntry, ids: &[&str]) -> Vec<String> {
    let mut problems: Vec<String> = task_id_problem(&entry.id).into_iter().collect();
    if entry.title.trim().is_empty() || entry.milestone.trim().is_empty() {
        problems.push(format!(
            "backlog.yaml: {} needs a title and a milestone",
            entry.id
        ));
    }
    for dependency in &entry.depends_on {
        if dependency == &entry.id || !ids.contains(&dependency.as_str()) {
            problems.push(format!(
                "{} depends on {dependency}, which is itself or not in the backlog",
                entry.id
            ));
        }
    }
    match (&entry.file, entry.status) {
        (Some(file), _) if !root.join(".development").join(file).is_file() => {
            problems.push(format!("{}: .development/{file} does not exist", entry.id));
        }
        (None, Status::InProgress | Status::Done | Status::Blocked) => {
            problems.push(format!(
                "{} has a status past todo but no task file",
                entry.id
            ));
        }
        (None, Status::Todo) | (Some(_), _) => {}
    }
    problems
}

fn unlisted_files(root: &Path, backlog: &Backlog) -> Result<Vec<String>, GateError> {
    let listed: BTreeSet<&str> = backlog
        .tasks
        .iter()
        .filter_map(|entry| entry.file.as_deref())
        .collect();
    let directory = root.join(".development/tasks");
    let mut problems = Vec::new();
    let entries = fs::read_dir(&directory).map_err(|source| GateError::Read {
        path: directory.clone(),
        source,
    })?;
    for entry in entries {
        let name = entry
            .map_err(|source| GateError::Read {
                path: directory.clone(),
                source,
            })?
            .file_name();
        let file = name.to_str().map(|text| format!("tasks/{text}"));
        let is_yaml = |path: &String| {
            Path::new(path)
                .extension()
                .is_some_and(|extension| extension == "yaml")
        };
        if let Some(path) = file.filter(|path| is_yaml(path) && !listed.contains(path.as_str())) {
            problems.push(format!(".development/{path} is not listed in backlog.yaml"));
        }
    }
    Ok(problems)
}

fn cycles(backlog: &Backlog) -> Vec<String> {
    let graph: BTreeMap<&str, Vec<&str>> = backlog
        .tasks
        .iter()
        .map(|entry| {
            (
                entry.id.as_str(),
                entry.depends_on.iter().map(String::as_str).collect(),
            )
        })
        .collect();
    let mut problems = Vec::new();
    for (start, dependencies) in &graph {
        let mut stack = dependencies.clone();
        let mut seen = BTreeSet::new();
        while let Some(current) = stack.pop() {
            if current == *start {
                problems.push(format!("dependency cycle through {start}"));
                break;
            }
            if seen.insert(current) {
                stack.extend(graph.get(current).into_iter().flatten());
            }
        }
    }
    problems
}

pub(crate) fn problems(root: &Path, backlog: &Backlog) -> Result<Vec<String>, GateError> {
    let ids: Vec<&str> = backlog
        .tasks
        .iter()
        .map(|entry| entry.id.as_str())
        .collect();
    let mut problems: Vec<String> = duplicates(&ids)
        .into_iter()
        .map(|id| format!("backlog.yaml lists {id} twice"))
        .collect();
    problems.extend(
        backlog
            .tasks
            .iter()
            .flat_map(|entry| entry_problems(root, entry, &ids)),
    );
    problems.extend(unlisted_files(root, backlog)?);
    problems.extend(cycles(backlog));
    Ok(problems)
}
