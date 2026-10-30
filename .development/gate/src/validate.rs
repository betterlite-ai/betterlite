mod agents;
pub(crate) mod backlog;
pub(crate) mod catalog;
mod command_args;
mod needs;
mod rules;
mod task;
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::de::DeserializeOwned;

use crate::backlog::read_backlog;
use crate::coverage;
use crate::deviations;
use crate::error::GateError;
use crate::schema;
use crate::spec::agents::Agents;
use crate::spec::backlog::Backlog;
use crate::spec::catalog::Catalog;
use crate::spec::litellm::Litellm;
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

struct Specs {
    catalog: Catalog,
    backlog: Backlog,
    rules: Rules,
    agents: Agents,
    litellm: Litellm,
}

fn load_specs(folder: &Path, problems: &mut Vec<String>) -> Option<Specs> {
    let catalog = load(&folder.join("checks.yaml"), problems);
    let backlog = match read_backlog(folder) {
        Ok((backlog, found)) => {
            problems.extend(found);
            Some(backlog)
        }
        Err(error) => {
            problems.push(error.to_string());
            None
        }
    };
    let rules = load(&folder.join("rules.yaml"), problems);
    let agents = load(&folder.join("agents.yaml"), problems);
    let litellm = load(&folder.join("litellm.yaml"), problems);
    Some(Specs {
        catalog: catalog?,
        backlog: backlog?,
        rules: rules?,
        agents: agents?,
        litellm: litellm?,
    })
}

fn pin_problems(litellm: &Litellm) -> Vec<String> {
    let mut problems = Vec::new();
    if !litellm.repository.starts_with("https://")
        || litellm.version.trim().is_empty()
        || litellm.roots.is_empty()
    {
        problems.push(
            "litellm.yaml needs an https repository, a version and at least one root".to_owned(),
        );
    }
    let unexplained = litellm
        .excluded
        .iter()
        .filter(|exclusion| exclusion.why.trim().is_empty());
    problems.extend(
        unexplained
            .map(|exclusion| format!("litellm.yaml: exclusion {} needs a why", exclusion.glob)),
    );
    problems
}

pub(crate) fn validate(root: &Path) -> Result<Vec<String>, GateError> {
    let folder = root.join(".development");
    let mut problems = Vec::new();
    let Some(Specs {
        catalog,
        backlog,
        rules,
        agents,
        litellm,
    }) = load_specs(&folder, &mut problems)
    else {
        return Ok(problems);
    };
    problems.extend(pin_problems(&litellm));
    problems.extend(catalog::problems(&catalog));
    problems.extend(backlog::problems(root, &backlog)?);
    problems.extend(rules::problems(&rules, &catalog));
    problems.extend(agents::problems(&folder, &agents));
    problems.extend(coverage::problems(&coverage::measure(
        &folder, &litellm, &backlog,
    )?));
    let mut tasks = BTreeMap::new();
    for entry in &backlog.tasks {
        if let Some(file) = &entry.file {
            let parsed: Option<Task> = load(&folder.join(file), &mut problems);
            if let Some(spec) = parsed {
                problems.extend(task::problems(root, &catalog, entry, &spec));
                tasks.insert(entry.id.clone(), spec);
            }
        }
    }
    problems.extend(needs::problems(root, &catalog, &backlog, &tasks)?);
    problems.extend(deviations::log_problems(root, &backlog)?);
    problems.extend(schema::stale(root)?);
    Ok(problems)
}

pub(crate) fn duplicates<'ids>(ids: &[&'ids str]) -> BTreeSet<&'ids str> {
    let mut seen = BTreeSet::new();
    ids.iter().copied().filter(|id| !seen.insert(*id)).collect()
}
