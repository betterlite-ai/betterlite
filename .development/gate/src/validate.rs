mod agents;
mod backlog;
pub(crate) mod catalog;
mod rules;
mod task;
mod tests;

use std::collections::BTreeSet;
use std::path::Path;

use serde::de::DeserializeOwned;

use crate::error::GateError;
use crate::schema;
use crate::spec::agents::Agents;
use crate::spec::backlog::Backlog;
use crate::spec::catalog::Catalog;
use crate::spec::rules::Rules;
use crate::spec::task::Task;
use crate::yaml::read_yaml;

fn load<Spec>(path: &Path, problems: &mut Vec<String>) -> Option<Spec>
where
    Spec: DeserializeOwned,
{
    match read_yaml(path) {
        Ok(spec) => Some(spec),
        Err(error) => {
            problems.push(error.to_string());
            None
        }
    }
}

pub(crate) fn validate(root: &Path) -> Result<Vec<String>, GateError> {
    let folder = root.join(".development");
    let mut problems = Vec::new();
    let loaded: (
        Option<Catalog>,
        Option<Backlog>,
        Option<Rules>,
        Option<Agents>,
    ) = (
        load(&folder.join("checks.yaml"), &mut problems),
        load(&folder.join("backlog.yaml"), &mut problems),
        load(&folder.join("rules.yaml"), &mut problems),
        load(&folder.join("agents.yaml"), &mut problems),
    );
    let (Some(catalog), Some(backlog), Some(rules), Some(agents)) = loaded else {
        return Ok(problems);
    };
    problems.extend(catalog::problems(&catalog));
    problems.extend(backlog::problems(root, &backlog)?);
    problems.extend(rules::problems(&rules, &catalog));
    problems.extend(agents::problems(&folder, &agents));
    for entry in &backlog.tasks {
        if let Some(file) = &entry.file {
            let parsed: Option<Task> = load(&folder.join(file), &mut problems);
            problems.extend(
                parsed
                    .map(|spec| task::problems(&catalog, entry, &spec))
                    .unwrap_or_default(),
            );
        }
    }
    problems.extend(schema::stale(root)?);
    Ok(problems)
}

pub(crate) fn duplicates<'ids>(ids: &[&'ids str]) -> BTreeSet<&'ids str> {
    let mut seen = BTreeSet::new();
    ids.iter().copied().filter(|id| !seen.insert(*id)).collect()
}
