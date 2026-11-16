use indexmap::IndexMap;

use crate::auth_model_lists::access_groups::{AccessGroupScope, models_from_access_groups};
use crate::auth_model_lists::lists::dedup_first;
use crate::auth_model_patterns::{ALL_PROXY_MODELS, ALL_TEAM_MODELS};
#[must_use]
pub fn get_key_models(
    key_models: &[String],
    team_models: &[String],
    proxy_model_list: &[String],
    model_access_groups: &IndexMap<String, Vec<String>>,
    scope: AccessGroupScope,
) -> Vec<String> {
    let mut all_models: Vec<String> = Vec::new();
    if !key_models.is_empty() {
        all_models.extend(key_models.iter().cloned());
        if all_models.iter().any(|model| model == ALL_TEAM_MODELS) {
            all_models = team_models.to_vec();
            if all_models.iter().any(|model| model == ALL_TEAM_MODELS) {
                all_models.retain(|model| model != ALL_TEAM_MODELS);
                all_models.extend(proxy_model_list.iter().cloned());
                if scope.include {
                    all_models.extend(model_access_groups.keys().cloned());
                }
            }
        }
        if all_models.iter().any(|model| model == ALL_PROXY_MODELS) {
            all_models = proxy_model_list.to_vec();
            if scope.include {
                all_models.extend(model_access_groups.keys().cloned());
            }
        }
    }
    let expanded = models_from_access_groups(
        model_access_groups,
        &all_models,
        scope.include,
        proxy_model_list,
    );
    let unique_models = dedup_first(expanded);
    tracing::debug!(count = unique_models.len(), "ALL KEY MODELS");
    unique_models
}
