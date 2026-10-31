mod tests;

use std::fs;
use std::path::Path;

use crate::error::GateError;
use crate::spec::backlog::Backlog;
use crate::spec::deviations::DeviationLog;
use crate::yaml::read_yaml;

pub(crate) fn read_log(root: &Path, task_id: &str) -> Result<Option<DeviationLog>, GateError> {
    let path = root.join(format!("docs/deviations/{task_id}.yaml"));
    if !path.exists() {
        return Ok(None);
    }
    read_yaml(&path).map(Some)
}

pub(crate) fn file_problems(path: &Path, backlog: &Backlog) -> Vec<String> {
    let shown = path.display();
    let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
        return vec![format!("{shown}: the file name is not unicode")];
    };
    if path.extension().and_then(|extension| extension.to_str()) != Some("yaml") {
        return vec![format!("{shown}: deviation logs are <task id>.yaml files")];
    }
    let log: DeviationLog = match read_yaml(path) {
        Ok(log) => log,
        Err(error) => return vec![error.to_string()],
    };
    let mut problems = Vec::new();
    if log.task != stem {
        problems.push(format!("{shown} names task {}", log.task));
    }
    if !backlog.tasks.iter().any(|entry| entry.id == log.task) {
        problems.push(format!("{shown}: {} is not in the backlog", log.task));
    }
    let incomplete = log.deviations.iter().any(|entry| {
        entry.what.trim().is_empty()
            || entry.why.trim().is_empty()
            || !(entry.link.is_empty() || entry.link.starts_with("https://"))
    });
    if incomplete {
        problems.push(format!(
            "{shown}: every deviation needs what, why, and an https link or an empty one"
        ));
    }
    problems
}

pub(crate) fn log_problems(root: &Path, backlog: &Backlog) -> Result<Vec<String>, GateError> {
    let folder = root.join("docs/deviations");
    if !folder.exists() {
        return Ok(Vec::new());
    }
    let listing = fs::read_dir(&folder).map_err(|source| GateError::Read {
        path: folder.clone(),
        source,
    })?;
    let mut problems = Vec::new();
    for item in listing {
        let path = item
            .map_err(|source| GateError::Read {
                path: folder.clone(),
                source,
            })?
            .path();
        problems.extend(file_problems(&path, backlog));
    }
    Ok(problems)
}
