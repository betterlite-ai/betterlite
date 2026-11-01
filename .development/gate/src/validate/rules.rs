use itertools::Itertools as _;

use crate::ids::check_id_problem;
use crate::spec::catalog::Catalog;
use crate::spec::rules::Rules;
use crate::validate::catalog;

pub(crate) fn problems(rules: &Rules, catalog: &Catalog) -> Vec<String> {
    let ids: Vec<&str> = rules.rules.iter().map(|rule| rule.id.as_str()).collect();
    let mut problems: Vec<String> = ids
        .iter()
        .duplicates()
        .map(|id| format!("rules.yaml defines {id} twice"))
        .collect();
    let checks = catalog::ids(catalog);
    for rule in &rules.rules {
        let id = &rule.id;
        problems.extend(check_id_problem(id).map(|problem| format!("rules.yaml: {problem}")));
        let texts = [
            &rule.title,
            &rule.statement,
            &rule.why,
            &rule.example.instead_of,
            &rule.example.write,
        ];
        if texts.iter().any(|text| text.trim().is_empty()) {
            problems.push(format!(
                "rules.yaml: {id} needs a title, statement, why and both halves of its example"
            ));
        }
        for check in rule
            .enforced_by
            .iter()
            .filter(|check| !checks.contains(&check.as_str()))
        {
            problems.push(format!(
                "rules.yaml: {id} is enforced by {check}, which checks.yaml does not define"
            ));
        }
        if rule.enforced_by.is_empty() && !rule.reviewed {
            problems.push(format!(
                "rules.yaml: {id} has no check and no review, so nothing enforces it"
            ));
        }
    }
    problems
}
