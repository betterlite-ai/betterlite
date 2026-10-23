use std::fs;
use std::path::Path;

use crate::builtins::Context;
use crate::error::GateError;
use crate::git::git;

fn measured_paths(context: &Context<'_>, roots: &[&str]) -> Result<Vec<String>, GateError> {
    let mut arguments = vec!["ls-files", "-z", "--"];
    arguments.extend(roots.iter());
    let tracked = git(&context.development.root, &arguments)?;
    let is_rust = |path: &&str| {
        Path::new(path)
            .extension()
            .is_some_and(|extension| extension == "rs")
    };
    let under_root = |path: &&str| {
        roots
            .iter()
            .any(|directory| path.starts_with(&format!("{directory}/")))
    };
    let mut paths: Vec<String> = tracked
        .split('\0')
        .chain(context.facts.untracked.iter().map(String::as_str))
        .filter(is_rust)
        .filter(under_root)
        .filter(|path| context.development.root.join(path).exists())
        .map(str::to_owned)
        .collect();
    paths.sort_unstable();
    paths.dedup();
    Ok(paths)
}

pub(crate) fn check(context: &Context<'_>) -> Result<Vec<String>, GateError> {
    let root = &context.development.root;
    let settings = &context.development.catalog.settings.file_size;
    let roots: Vec<&str> = settings
        .roots
        .iter()
        .map(String::as_str)
        .filter(|directory| root.join(directory).exists())
        .collect();
    let mut problems = Vec::new();
    for path in measured_paths(context, &roots)? {
        let file = root.join(&path);
        let count = fs::read_to_string(&file)
            .map_err(|source| GateError::Read { path: file, source })?
            .lines()
            .count();
        let is_test = path.ends_with("/tests.rs") || format!("/{path}").contains("/tests/");
        let limit = if is_test {
            settings.test_max_lines
        } else {
            settings.src_max_lines
        };
        if count > limit {
            problems.push(format!(
                "{path} has {count} lines, limit is {limit}; split it along a real boundary"
            ));
        }
    }
    Ok(problems)
}
