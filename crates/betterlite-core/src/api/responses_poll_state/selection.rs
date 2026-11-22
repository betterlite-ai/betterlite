use betterlite_config::api::responses_poll_state::{PollingSelector, PollingSettings};

// the litellm_params fields the alias scan reads, without the router's own config types
#[derive(Debug, Clone, Copy)]
pub struct DeploymentProvider<'a> {
    pub custom_llm_provider: Option<&'a str>,
    pub model: Option<&'a str>,
}

#[derive(Debug, Clone, Copy)]
pub struct DeploymentProjection<'a> {
    pub indices: &'a [usize],
    pub deployments: &'a [DeploymentProvider<'a>],
}

#[must_use]
pub fn should_use_polling_for_request(
    background: bool,
    model: &str,
    settings: &PollingSettings,
    redis_available: bool,
    router: Option<&DeploymentProjection<'_>>,
) -> bool {
    let selector = &settings.polling_via_cache;
    let selector_active = !matches!(selector, PollingSelector::Disabled)
        && !matches!(selector, PollingSelector::Providers(providers) if providers.is_empty());
    if !(background && selector_active && redis_available) {
        return false;
    }
    if settings
        .native_background_mode
        .iter()
        .any(|native| native == model)
    {
        tracing::debug!(
            "Model {} is in native_background_mode list, skipping polling via cache",
            model
        );
        return false;
    }
    match selector {
        PollingSelector::All => true,
        PollingSelector::Disabled | PollingSelector::Unrecognized => false,
        PollingSelector::Providers(providers) => match model.split_once('/') {
            // a slash model only checks its prefix; it never falls through to the deployments
            Some((provider, _rest)) => providers.iter().any(|known| known == provider),
            None => router.is_some_and(|projection| scan_deployments(projection, providers, model)),
        },
    }
}

fn scan_deployments(
    projection: &DeploymentProjection<'_>,
    providers: &[String],
    model: &str,
) -> bool {
    for &index in projection.indices {
        let Some(deployment) = projection.deployments.get(index) else {
            tracing::debug!(
                "Could not resolve provider for model {}: list index out of range",
                model
            );
            return false;
        };
        let provider = deployment
            .custom_llm_provider
            .filter(|name| !name.is_empty());
        let candidate = provider.or_else(|| {
            deployment
                .model
                .and_then(|name| name.split_once('/'))
                .map(|(name, _rest)| name)
        });
        if let Some(provider) = candidate
            && providers.iter().any(|known| known == provider)
        {
            tracing::debug!("Polling enabled for model={}, provider={}", model, provider);
            return true;
        }
    }
    false
}
