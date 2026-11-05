use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use globset::GlobSet;
use serde::Deserialize;

use crate::builtins::globs::glob_set;
use crate::error::GateError;
use crate::spec::backlog::{Backlog, Status};
use crate::spec::catalog::Catalog;
use crate::spec::task::Task;
use crate::validate::command_args::filtered_runs;

#[derive(Deserialize)]
struct Manifest {
    package: Option<Package>,
}

#[derive(Deserialize)]
struct Package {
    name: String,
}

struct OpenModule {
    check: String,
    name: String,
    places: Vec<String>,
}

impl OpenModule {
    fn files(&self) -> Vec<String> {
        let name = &self.name;
        self.places
            .iter()
            .flat_map(|place| {
                [
                    format!("{place}/src/{name}.rs"),
                    format!("{place}/src/{name}/mod.rs"),
                ]
            })
            .collect()
    }

    fn problem(&self, id: &str, own: &GlobSet, dependencies: &GlobSet) -> Option<String> {
        let files = self.files();
        if files.iter().any(|file| dependencies.is_match(file)) {
            return None;
        }
        let creates = files.iter().any(|file| own.is_match(file));
        let declares = self.places.iter().any(|place| {
            own.is_match(format!("{place}/src/lib.rs"))
                || own.is_match(format!("{place}/src/main.rs"))
        });
        let (check, name) = (&self.check, &self.name);
        match (creates, declares) {
            (true, true) => None,
            (true, false) => Some(format!(
                "{id}: check {check} filters tests under {name}::, a module its scope creates but no crate root declares; the skeleton declares it before dispatch"
            )),
            (false, _) => Some(format!(
                "{id}: check {check} filters tests under {name}::, but {}/src has no module {name} and neither its scope nor an open dependency's creates one",
                self.places.join("/src, ")
            )),
        }
    }
}

fn package_folders(root: &Path) -> Result<BTreeMap<String, String>, GateError> {
    let mut folders = BTreeMap::new();
    for group in ["crates", "tools"] {
        let parent = root.join(group);
        let listing = fs::read_dir(&parent).map_err(|source| GateError::Read {
            path: parent.clone(),
            source,
        })?;
        for listed in listing {
            let member = listed.map_err(|source| GateError::Read {
                path: parent.clone(),
                source,
            })?;
            let path = member.path().join("Cargo.toml");
            let file_name = member.file_name();
            let Some(folder) = file_name.to_str().filter(|_| path.is_file()) else {
                continue;
            };
            let text = fs::read_to_string(&path).map_err(|source| GateError::Read {
                path: path.clone(),
                source,
            })?;
            let manifest: Manifest = toml::from_str(&text).map_err(|source| GateError::Toml {
                path: path.clone(),
                source: Box::new(source),
            })?;
            if let Some(package) = manifest.package {
                folders.insert(package.name, format!("{group}/{folder}"));
            }
        }
    }
    Ok(folders)
}

// a nextest filter on a module that no crate declares matches nothing, and --no-tests=fail fails it
fn open_modules(root: &Path, folders: &BTreeMap<String, String>, task: &Task) -> Vec<OpenModule> {
    let mut open = Vec::new();
    for check in &task.checks.commands {
        for run in filtered_runs(&check.run) {
            let Some(places): Option<Vec<String>> = run
                .packages
                .iter()
                .map(|package| folders.get(package).cloned())
                .collect()
            else {
                continue;
            };
            let writes_tests = task.scope.paths.iter().any(|path| {
                places
                    .iter()
                    .any(|place| path.starts_with(&format!("{place}/tests/")))
            });
            if writes_tests {
                continue;
            }
            let modules = run.modules.into_iter().map(|name| OpenModule {
                check: check.id.clone(),
                name,
                places: places.clone(),
            });
            open.extend(
                modules
                    .filter(|module| !module.files().iter().any(|file| root.join(file).is_file())),
            );
        }
    }
    open
}

// a folder (a `test -d` path or an oracle case folder) counts as covered when a file inside it would be
pub(crate) fn needed_paths(task: &Task) -> Vec<(String, bool)> {
    let mut needed: Vec<(String, bool)> = task
        .oracle_cases
        .iter()
        .map(|case| {
            let folder = !Path::new(case)
                .extension()
                .is_some_and(|extension| extension == "yaml" || extension == "yml");
            (case.clone(), folder)
        })
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
        needed.extend(added_file_pathspecs(&words));
    }
    needed
}

// a check that wants new files under a pathspec can only pass when the task may write there
fn added_file_pathspecs(words: &[&str]) -> Vec<(String, bool)> {
    let mut found = Vec::new();
    let mut wants_added = false;
    let mut in_pathspecs = false;
    for word in words {
        let path = word.trim_end_matches(')');
        match *word {
            "git" | "&&" | "||" | ";" | "|" => {
                wants_added = false;
                in_pathspecs = false;
            }
            "--diff-filter=A" => wants_added = true,
            "--" if wants_added => in_pathspecs = true,
            _ if in_pathspecs && !path.is_empty() && !path.starts_with(['$', ':']) => {
                if let Some((prefix, _)) = path.split_once(['*', '?', '[']) {
                    found.push((format!("{prefix}probe"), false));
                } else {
                    let file = path
                        .rsplit('/')
                        .next()
                        .is_some_and(|last| last.contains('.'));
                    found.push((path.trim_end_matches('/').to_owned(), !file));
                }
                in_pathspecs = !word.ends_with(')');
            }
            _ => {}
        }
    }
    found
}

// a done dependency is on main already, so its scope adds no file that main lacks
fn open_dependency_scopes(
    backlog: &Backlog,
    tasks: &BTreeMap<String, Task>,
    id: &str,
) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut scopes = Vec::new();
    let mut stack = vec![id];
    while let Some(current) = stack.pop() {
        let Some(entry) = backlog.tasks.iter().find(|entry| entry.id == current) else {
            continue;
        };
        if current != id
            && entry.status != Status::Done
            && let Some(task) = tasks.get(current)
        {
            scopes.extend(task.scope.paths.iter().cloned());
        }
        for dependency in &entry.depends_on {
            if seen.insert(dependency.as_str()) {
                stack.push(dependency.as_str());
            }
        }
    }
    scopes
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
    let folders = package_folders(root)?;
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
        let own = glob_set(&allowed)?;
        let open = open_needs(root, task, &own);
        let modules = if entry.status == Status::Todo {
            open_modules(root, &folders, task)
        } else {
            Vec::new()
        };
        if open.is_empty() && modules.is_empty() {
            continue;
        }
        let dependencies = glob_set(&open_dependency_scopes(backlog, tasks, &entry.id))?;
        for (path, probe) in open {
            if !dependencies.is_match(&probe) {
                problems.push(format!(
                    "{}: needs {path}, which is neither in its scope or an open dependency's nor on main",
                    entry.id
                ));
            }
        }
        problems.extend(
            modules
                .iter()
                .filter_map(|module| module.problem(&entry.id, &own, &dependencies)),
        );
    }
    Ok(problems)
}
