use std::collections::BTreeSet;

use indexmap::IndexMap;

use crate::auth_model_lists::access_groups::models_from_access_groups;
use crate::auth_model_lists::lists::{ALL_PROXY_MODELS, ALL_TEAM_MODELS, dedup_first};

// get_team_models: LiteLLM builds a Python set, so the listed order is sorted here and never inherited.
#[must_use]
pub fn get_team_models(
    team_models: &[String],
    proxy_model_list: &[String],
    model_access_groups: &IndexMap<String, Vec<String>>,
    include_model_access_groups: bool,
) -> Vec<String> {
    let mut all_models_set: BTreeSet<String> = BTreeSet::new();
    if !team_models.is_empty() {
        all_models_set.extend(team_models.iter().cloned());
        if all_models_set.contains(ALL_TEAM_MODELS) {
            all_models_set.remove(ALL_TEAM_MODELS);
            all_models_set.extend(proxy_model_list.iter().cloned());
            if include_model_access_groups {
                all_models_set.extend(model_access_groups.keys().cloned());
            }
        }
        if all_models_set.contains(ALL_PROXY_MODELS) {
            all_models_set.extend(proxy_model_list.iter().cloned());
            if include_model_access_groups {
                all_models_set.extend(model_access_groups.keys().cloned());
            }
        }
    }
    let listed: Vec<String> = all_models_set.into_iter().collect();
    let expanded = models_from_access_groups(
        model_access_groups,
        &listed,
        include_model_access_groups,
        proxy_model_list,
    );
    let unique_models = dedup_first(expanded);
    tracing::debug!(count = unique_models.len(), "ALL TEAM MODELS");
    unique_models
}
