mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use itertools::Itertools as _;

use crate::error::GateError;
use crate::ids::task_id_problem;
use crate::spec::backlog::{Backlog, BacklogEntry, Resolution, Status};

fn link_problems(entry: &BacklogEntry, ids: &[&str]) -> Vec<String> {
    let mut problems = Vec::new();
    let links = [
        ("parent", &entry.parent),
        ("split_from", &entry.split_from),
        ("superseded_by", &entry.superseded_by),
    ];
    for (field, link) in links {
        if let Some(target) = link
            && (target == &entry.id || !ids.contains(&target.as_str()))
        {
            problems.push(format!(
                "{}: {field} {target} is itself or not in the backlog",
                entry.id
            ));
        }
    }
    problems
}

pub(crate) fn ticket_problems(entry: &BacklogEntry, ids: &[&str]) -> Vec<String> {
    let mut problems = link_problems(entry, ids);
    if (entry.status == Status::Done) != entry.resolution.is_some() {
        problems.push(format!(
            "{}: a done entry has a resolution, and only a done entry has one",
            entry.id
        ));
    }
    let replaced = matches!(
        entry.resolution,
        Some(Resolution::Duplicate | Resolution::Superseded)
    );
    if replaced != entry.superseded_by.is_some() {
        problems.push(format!(
            "{}: superseded_by is set exactly when the resolution is duplicate or superseded",
            entry.id
        ));
    }
    if entry.resolution == Some(Resolution::Merged) && entry.pr.is_none() {
        problems.push(format!("{}: a merged entry names its pr", entry.id));
    }
    let reason = entry
        .blocked_reason
        .as_deref()
        .is_some_and(|text| !text.trim().is_empty());
    if (entry.status == Status::Blocked) != reason {
        problems.push(format!(
            "{}: blocked_reason is set exactly when the status is blocked",
            entry.id
        ));
    }
    problems
}

fn parent_cycles(backlog: &Backlog) -> Vec<String> {
    let parents: BTreeMap<&str, &str> = backlog
        .tasks
        .iter()
        .filter_map(|entry| {
            entry
                .parent
                .as_deref()
                .map(|parent| (entry.id.as_str(), parent))
        })
        .collect();
    let mut problems = Vec::new();
    for start in parents.keys() {
        let mut current = *start;
        for _ in 0..parents.len() {
            let Some(next) = parents.get(current) else {
                break;
            };
            if next == start {
                problems.push(format!("{start} is its own ancestor through parent"));
                break;
            }
            current = next;
        }
    }
    problems
}

fn entry_problems(root: &Path, entry: &BacklogEntry, ids: &[&str]) -> Vec<String> {
    let mut problems: Vec<String> = task_id_problem(&entry.id).into_iter().collect();
    if entry.title.trim().is_empty() || entry.milestone.trim().is_empty() {
        problems.push(format!(
            "backlog/{}.yaml needs a title and a milestone",
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
        (None, Status::Done) if entry.resolution != Some(Resolution::Merged) => {}
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
            problems.push(format!(".development/{path} has no backlog entry"));
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
    let mut problems: Vec<String> = ids
        .iter()
        .duplicates()
        .map(|id| format!("the backlog lists {id} twice"))
        .collect();
    problems.extend(
        backlog
            .tasks
            .iter()
            .flat_map(|entry| entry_problems(root, entry, &ids)),
    );
    problems.extend(
        backlog
            .tasks
            .iter()
            .flat_map(|entry| ticket_problems(entry, &ids)),
    );
    problems.extend(parent_cycles(backlog));
    problems.extend(unlisted_files(root, backlog)?);
    problems.extend(cycles(backlog));
    Ok(problems)
}
