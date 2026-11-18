use std::collections::BTreeSet;

use betterlite_config::spend_attribution::Settings as SpendAttributionSettings;
use betterlite_models::resolution::ResolutionContext;
use betterlite_provider::identity::spend_proxy::{
    DeploymentIdentity, ProxySelection, deployment_provider,
};

#[must_use]
pub fn model_group_provider(
    model_group: &str,
    deployments: &[DeploymentIdentity<'_>],
    settings: &SpendAttributionSettings,
    resolution: &ResolutionContext<'_>,
) -> Option<String> {
    if model_group.is_empty() {
        return None;
    }
    let selection = ProxySelection {
        environment: settings.environment_use_litellm_proxy,
        global: settings.global_use_litellm_proxy,
    };
    let mut provider: Option<String> = None;
    for deployment in deployments {
        let Some(resolved) = deployment_provider(deployment, &selection, resolution) else {
            continue;
        };
        if provider.as_ref().is_some_and(|known| known != &resolved) {
            return None;
        }
        provider = Some(resolved);
    }
    provider
}

#[must_use]
pub fn configured_model_group(
    model_group: &str,
    recognized_model_groups: &BTreeSet<String>,
    team_public_model_names: &BTreeSet<String>,
) -> bool {
    !model_group.is_empty()
        && (recognized_model_groups.contains(model_group)
            || team_public_model_names.contains(model_group))
}

#[must_use]
pub fn model_name_is_valid(
    model: &str,
    recognized_model_groups: &BTreeSet<String>,
    team_public_model_names: &BTreeSet<String>,
) -> bool {
    configured_model_group(model, recognized_model_groups, team_public_model_names)
        || betterlite_logging::spend_metadata::looks_like_model_name(model)
}

#[cfg(test)]
mod tests;
