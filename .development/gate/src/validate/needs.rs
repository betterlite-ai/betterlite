use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use globset::GlobSet;

use crate::builtins::globs::glob_set;
use crate::error::GateError;
use crate::spec::backlog::{Backlog, Status};
use crate::spec::catalog::Catalog;
use crate::spec::task::Task;

// a `test -d` path counts as covered when a file inside it would be
pub(crate) fn needed_paths(task: &Task) -> Vec<(String, bool)> {
    let mut needed: Vec<(String, bool)> = task
        .oracle_cases
        .iter()
        .map(|case| (case.clone(), false))
        .collect();
    for check in &task.checks.commands {
        let words: Vec<&str> = check
            .run
            .split(|letter: char| letter.is_whitespace() || "'\"".contains(letter))
            .filter(|word| !word.is_empty())
            .collect();
        for window in words.windows(3) {
            if let [command, flag, path] = window
                && (*command == "test" || *command == "[")
                && (*flag == "-f" || *flag == "-d")
                && !path.starts_with('$')
            {
                needed.push(((*path).to_owned(), *flag == "-d"));
            }
        }
    }
    needed
}

fn dependency_closure<'backlog>(backlog: &'backlog Backlog, id: &str) -> BTreeSet<&'backlog str> {
    let mut seen = BTreeSet::new();
    let mut stack = vec![id];
    while let Some(current) = stack.pop() {
        let Some(entry) = backlog.tasks.iter().find(|entry| entry.id == current) else {
            continue;
        };
        for dependency in &entry.depends_on {
            if seen.insert(dependency.as_str()) {
                stack.push(dependency.as_str());
            }
        }
    }
    seen
}

fn open_needs(root: &Path, task: &Task, own: &GlobSet) -> Vec<(String, String)> {
    let mut open = Vec::new();
    for (path, directory) in needed_paths(task) {
        let probe = if directory {
            format!("{path}/probe")
        } else {
            path.clone()
        };
        if !root.join(&path).exists() && !own.is_match(&probe) {
            open.push((path, probe));
        }
    }
    open
}

pub(crate) fn problems(
    root: &Path,
    catalog: &Catalog,
    backlog: &Backlog,
    tasks: &BTreeMap<String, Task>,
) -> Result<Vec<String>, GateError> {
    let mut problems = Vec::new();
    for entry in backlog
        .tasks
        .iter()
        .filter(|entry| entry.status != Status::Done)
    {
        let Some(task) = tasks.get(&entry.id) else {
            continue;
        };
        let mut allowed = task.scope.paths.clone();
        allowed.extend(catalog.settings.always_in_scope.iter().cloned());
        let open = open_needs(root, task, &glob_set(&allowed)?);
        if open.is_empty() {
            continue;
        }
        for dependency in dependency_closure(backlog, &entry.id) {
            if let Some(other) = tasks.get(dependency) {
                allowed.extend(other.scope.paths.iter().cloned());
            }
        }
        let owned = glob_set(&allowed)?;
        for (path, probe) in open {
            if !owned.is_match(&probe) {
                problems.push(format!(
                    "{}: needs {path}, which is neither in its scope or a dependency's nor on main",
                    entry.id
                ));
            }
        }
    }
    Ok(problems)
}
