use std::collections::BTreeSet;

use betterlite_router::deployment_identity::spec::DeploymentSpec;

// _deduplicate_litellm_router_models: the first deployment under each model_info.id, and no deployment without one.
#[must_use]
pub fn deduplicate_router_models(models: &[DeploymentSpec]) -> Vec<DeploymentSpec> {
    let mut seen = BTreeSet::new();
    let mut unique = Vec::with_capacity(models.len());
    for model in models {
        let Some(id) = model.id() else {
            continue;
        };
        if !seen.insert(id.to_owned()) {
            continue;
        }
        unique.push(model.clone());
    }
    unique
}
