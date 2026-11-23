use betterlite_config::api::responses_poll_state::{PollingSelector, PollingSettings};

// the request facts the gate reads; redis_available is whether a cache is configured
#[derive(Debug, Clone, Copy)]
pub struct PollingRequest<'request> {
    pub background: bool,
    pub model: &'request str,
    pub redis_available: bool,
}

// the litellm_params fields the alias scan reads, without the router's own config types
#[derive(Debug, Clone, Copy)]
pub struct DeploymentProvider<'deployment> {
    pub custom_llm_provider: Option<&'deployment str>,
    pub model: Option<&'deployment str>,
}

#[derive(Debug, Clone, Copy)]
pub struct DeploymentProjection<'router> {
    pub indices: &'router [usize],
    pub deployments: &'router [DeploymentProvider<'router>],
}

#[must_use]
pub fn should_use_polling_for_request(
    request: PollingRequest<'_>,
    settings: &PollingSettings,
    router: Option<&DeploymentProjection<'_>>,
) -> bool {
    let selector = &settings.polling_via_cache;
    let selector_active = !matches!(selector, PollingSelector::Disabled)
        && !matches!(selector, PollingSelector::Providers(providers) if providers.is_empty());
    if !(request.background && selector_active && request.redis_available) {
        return false;
    }
    if settings
        .native_background_mode
        .iter()
        .any(|native| native == request.model)
    {
        tracing::debug!(
            "Model {} is in native_background_mode list, skipping polling via cache",
            request.model
        );
        return false;
    }
    match selector {
        PollingSelector::All => true,
        PollingSelector::Disabled | PollingSelector::Unrecognized => false,
        PollingSelector::Providers(providers) => match request.model.split_once('/') {
            // a slash model only checks its prefix; it never falls through to the deployments
            Some((provider, _rest)) => providers.iter().any(|known| known == provider),
            None => router
                .is_some_and(|projection| scan_deployments(projection, providers, request.model)),
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
        let custom = deployment
            .custom_llm_provider
            .filter(|name| !name.is_empty());
        let candidate = custom.or_else(|| {
            deployment
                .model
                .and_then(|name| name.split_once('/'))
                .map(|(name, _rest)| name)
        });
        if let Some(matched) = candidate
            && providers.iter().any(|known| known == matched)
        {
            tracing::debug!("Polling enabled for model={}, provider={}", model, matched);
            return true;
        }
    }
    false
}
