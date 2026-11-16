use std::collections::BTreeSet;

use indexmap::IndexMap;

// get_key_models and get_team_models read `include`, get_complete_model_list reads `only` as well.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AccessGroupScope {
    pub include: bool,
    pub only: bool,
}

// _get_models_from_access_groups: a listed group name stays only when it is also a deployment,
// and every group a listed name names contributes all of its members, in input order.
#[must_use]
pub fn models_from_access_groups(
    model_access_groups: &IndexMap<String, Vec<String>>,
    all_models: &[String],
    include: bool,
    proxy_model_list: &[String],
) -> Vec<String> {
    let deployed: BTreeSet<&str> = proxy_model_list.iter().map(String::as_str).collect();
    let mut expanded: Vec<String> = Vec::new();
    for model in all_models {
        let names_a_group = model_access_groups.contains_key(model);
        if !names_a_group || include || deployed.contains(model.as_str()) {
            expanded.push(model.clone());
        }
    }
    for model in all_models {
        if let Some(members) = model_access_groups.get(model) {
            expanded.extend(members.iter().cloned());
        }
    }
    expanded
}
