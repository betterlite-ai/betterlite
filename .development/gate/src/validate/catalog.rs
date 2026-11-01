use itertools::Itertools as _;

use crate::builtins::globs::glob_set;
use crate::ids::check_id_problem;
use crate::spec::catalog::Catalog;

pub(crate) fn ids(catalog: &Catalog) -> Vec<&str> {
    let builtin = catalog
        .base
        .builtin
        .iter()
        .map(|check| <&str>::from(check.id));
    let commands = catalog
        .base
        .commands
        .iter()
        .chain(&catalog.optional)
        .map(|check| check.id.as_str());
    builtin.chain(commands).collect()
}

pub(crate) fn problems(catalog: &Catalog) -> Vec<String> {
    let mut problems: Vec<String> = ids(catalog)
        .into_iter()
        .duplicates()
        .map(|id| format!("checks.yaml defines {id} twice"))
        .collect();
    problems.extend(
        catalog
            .base
            .commands
            .iter()
            .chain(&catalog.optional)
            .filter_map(|check| check_id_problem(&check.id))
            .map(|problem| format!("checks.yaml: {problem}")),
    );
    if catalog.optional.iter().all(|check| check.id != "oracle") {
        problems.push("checks.yaml needs an optional check with id oracle".to_owned());
    }
    for patterns in [
        &catalog.settings.always_in_scope,
        &catalog.settings.budget_excludes,
    ] {
        if let Err(error) = glob_set(patterns) {
            problems.push(format!("checks.yaml: {error}"));
        }
    }
    problems
}
