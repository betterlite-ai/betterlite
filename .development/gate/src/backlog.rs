mod tests;

use std::fs;
use std::path::Path;

use crate::error::GateError;
use crate::spec::backlog::{Backlog, BacklogEntry};
use crate::yaml::read_yaml;

fn read_entry(path: &Path, problems: &mut Vec<String>) -> Option<BacklogEntry> {
    let shown = path.display();
    if path.extension().and_then(|extension| extension.to_str()) != Some("yaml") {
        problems.push(format!("{shown}: backlog entries are <id>.yaml files"));
        return None;
    }
    let entry: BacklogEntry = match read_yaml(path) {
        Ok(entry) => entry,
        Err(error) => {
            problems.push(error.to_string());
            return None;
        }
    };
    if path.file_stem().and_then(|stem| stem.to_str()) != Some(entry.id.as_str()) {
        problems.push(format!(
            "{shown} holds {0}, so it must be named {0}.yaml",
            entry.id
        ));
    }
    Some(entry)
}

pub(crate) fn read_backlog(development: &Path) -> Result<(Backlog, Vec<String>), GateError> {
    let folder = development.join("backlog");
    let listing = fs::read_dir(&folder).map_err(|source| GateError::Read {
        path: folder.clone(),
        source,
    })?;
    let mut tasks = Vec::new();
    let mut problems = Vec::new();
    for item in listing {
        let path = item
            .map_err(|source| GateError::Read {
                path: folder.clone(),
                source,
            })?
            .path();
        tasks.extend(read_entry(&path, &mut problems));
    }
    tasks.sort_by_key(|entry| {
        entry
            .id
            .get(1..)
            .and_then(|digits| digits.parse::<u64>().ok())
            .unwrap_or(u64::MAX)
    });
    Ok((Backlog { tasks }, problems))
}
