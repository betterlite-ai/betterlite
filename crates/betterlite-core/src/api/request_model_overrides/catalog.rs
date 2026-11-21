#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeploymentParams {
    pub custom_llm_provider: Option<String>,
    pub model: Option<String>,
}

// the three Router indices the stale-alias check and _resolve_provider_from_deployment read
pub trait DeploymentCatalogView: Send + Sync {
    fn has_model_group(&self, model_group: &str) -> bool;
    fn has_team_model_group(&self, team_id: &str, model_group: &str) -> bool;
    fn deployment_params(&self, model_group: &str) -> Option<DeploymentParams>;
}

// the provider the caller's model name already carries, else the one the deployment resolves to
pub(crate) fn provider_hint(
    model: &str,
    pre_alias_model: Option<&str>,
    catalog: Option<&dyn DeploymentCatalogView>,
) -> Option<String> {
    if let Some((provider, _rest)) = model.split_once('/') {
        return (!provider.is_empty()).then(|| provider.to_owned());
    }
    let catalog = catalog?;
    let mut candidates = vec![model];
    if let Some(pre_alias) = pre_alias_model.filter(|pre_alias| *pre_alias != model) {
        candidates.push(pre_alias);
    }
    for name in candidates {
        let Some(params) = catalog.deployment_params(name) else {
            continue;
        };
        if let Some(provider) = params
            .custom_llm_provider
            .filter(|provider| !provider.is_empty())
        {
            return Some(provider);
        }
        // getattr(litellm_params, "model", "") told apart from a model with no provider prefix
        let Some((provider, _rest)) = params
            .model
            .as_deref()
            .and_then(|deployment_model| deployment_model.split_once('/'))
        else {
            continue;
        };
        return (!provider.is_empty()).then(|| provider.to_owned());
    }
    None
}
