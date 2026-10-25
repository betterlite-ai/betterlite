mod tests;

use crate::builtins::globs::glob_set;
use crate::development::Development;
use crate::error::GateError;
use crate::git::{common_dir, git};
use crate::spec::backlog::Status;
use crate::spec::task::Task;

pub(crate) fn overlaps(development: &Development, task: &Task) -> Result<Vec<String>, GateError> {
    let shared = glob_set(&development.catalog.settings.always_in_scope)?;
    let tracked = git(&development.root, &["ls-files"])?;
    let own = glob_set(&task.scope.paths)?;
    let own_probes = probes(&task.scope.paths);
    let journals = common_dir(&development.root)?.join("betterlite");
    let mut clashes = Vec::new();
    for entry in &development.backlog.tasks {
        // a journal on this machine means a dispatch ran here and its branch is not merged yet
        let running = entry.status == Status::InProgress
            || (entry.status != Status::Done && journals.join(&entry.id).is_dir());
        if entry.id == task.id || !running || entry.file.is_none() {
            continue;
        }
        let other = development.task(&entry.id)?;
        let theirs = glob_set(&other.scope.paths)?;
        let other_probes = probes(&other.scope.paths);
        let clash = tracked
            .lines()
            .chain(own_probes.iter().map(String::as_str))
            .chain(other_probes.iter().map(String::as_str))
            .find(|path| !shared.is_match(path) && own.is_match(path) && theirs.is_match(path));
        if let Some(path) = clash {
            clashes.push(format!(
                "{} is in progress and also writes {path}",
                entry.id
            ));
        }
    }
    Ok(clashes)
}

// a path inside every pattern, so two scopes that would both create the same new file still meet
pub(crate) fn probes(patterns: &[String]) -> Vec<String> {
    let mut expanded = Vec::new();
    for pattern in patterns {
        let braces = pattern.split_once('{').and_then(|(head, rest)| {
            rest.split_once('}')
                .map(|(inner, tail)| (head, inner, tail))
        });
        match braces {
            Some((head, alternatives, tail)) => {
                for alternative in alternatives.split(',') {
                    expanded.push(format!("{head}{alternative}{tail}"));
                }
            }
            None => expanded.push(pattern.clone()),
        }
    }
    expanded
        .into_iter()
        .map(|pattern| match pattern.split_once(['*', '?', '[']) {
            Some((prefix, _)) => format!("{prefix}probe"),
            None => pattern,
        })
        .collect()
}
