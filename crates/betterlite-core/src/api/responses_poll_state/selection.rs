use betterlite_config::api::responses_poll_state::{PollingSelector, PollingSettings};

#[derive(Debug, Clone, Copy)]
pub struct PollingRequest<'a> {
    pub background: bool,
    pub model: &'a str,
}

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
    request: PollingRequest<'_>,
    settings: &PollingSettings,
    redis_available: bool,
    router: Option<&DeploymentProjection<'_>>,
) -> bool {
    let selector = &settings.polling_via_cache;
    let selector_active = match selector {
        PollingSelector::Disabled => false,
        PollingSelector::All | PollingSelector::Unrecognized => true,
        PollingSelector::Providers(providers) => !providers.is_empty(),
    };
    if !(request.background && selector_active && redis_available) {
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
            None => match router {
                Some(projection) => scan_deployments(projection, providers, request.model),
                None => false,
            },
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
        let candidate = deployment
            .custom_llm_provider
            .filter(|provider| !provider.is_empty())
            .or_else(|| {
                deployment
                    .model
                    .and_then(|deployment_model| deployment_model.split_once('/'))
                    .map(|(provider, _rest)| provider)
            });
        if let Some(provider) = candidate {
            if providers.iter().any(|known| known == provider) {
                tracing::debug!("Polling enabled for model={}, provider={}", model, provider);
                return true;
            }
        }
    }
    false
}
