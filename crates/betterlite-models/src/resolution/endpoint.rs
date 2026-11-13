use std::collections::BTreeMap;

use secrecy::SecretString;

use crate::identity::endpoint::endpoint_matches_api_base;
use crate::provider_defaults_a::environment::first_set;

use super::{ProviderIdentity, ResolutionContext, ResolutionError, ResolutionRequest};
use table::{ENDPOINTS, ENV_BRANCHES, EXPLICIT_BRANCHES, EndpointProvider, HTTPS_ONLY_BRANCHES};

mod table;

pub(super) struct EndpointMatch {
    pub provider: ProviderIdentity,
    pub api_key: Option<SecretString>,
}

// Ports get_llm_provider lines 260-409: the first matching registered endpoint decides the provider.
pub(super) fn resolve(
    api_base: &str,
    request: &ResolutionRequest<'_>,
    context: &ResolutionContext<'_>,
) -> Result<Option<EndpointMatch>, ResolutionError> {
    let Some(endpoint) = ENDPOINTS
        .iter()
        .copied()
        .find(|endpoint| endpoint_matches_api_base(endpoint, api_base))
    else {
        return Ok(None);
    };
    let environment = context.environment;
    if let Some((named, api_key)) = known_branch(endpoint, api_base, request, environment) {
        let provider = match named {
            EndpointProvider::Anyscale => ProviderIdentity::Anyscale,
            EndpointProvider::Known(known) => ProviderIdentity::Known(known),
        };
        return Ok(Some(EndpointMatch { provider, api_key }));
    }
    if let Some(descriptor) = context.registry.get_by_base_url(endpoint) {
        let api_key = request.api_key.cloned().or_else(|| {
            first_set(environment, &[descriptor.config.api_key_env.as_str()])
                .map(SecretString::from)
        });
        let provider = ProviderIdentity::Known(descriptor.provider);
        return Ok(Some(EndpointMatch { provider, api_key }));
    }
    Err(ResolutionError::EndpointWithoutProvider {
        endpoint: endpoint.to_owned(),
        api_base: api_base.to_owned(),
    })
}

fn known_branch(
    endpoint: &str,
    api_base: &str,
    request: &ResolutionRequest<'_>,
    environment: &BTreeMap<String, String>,
) -> Option<(EndpointProvider, Option<SecretString>)> {
    if let Some((_, provider, names)) = ENV_BRANCHES.iter().find(|(name, ..)| *name == endpoint) {
        let api_key = first_set(environment, names).map(SecretString::from);
        return Some((*provider, api_key));
    }
    if let Some((_, provider, names)) = HTTPS_ONLY_BRANCHES
        .iter()
        .find(|(name, ..)| *name == endpoint)
    {
        let api_key = api_base
            .to_lowercase()
            .starts_with("https://")
            .then(|| first_set(environment, names))
            .flatten()
            .map(SecretString::from);
        return Some((*provider, api_key));
    }
    let (_, provider, names) = EXPLICIT_BRANCHES
        .iter()
        .find(|(name, ..)| *name == endpoint)?;
    let api_key = request
        .api_key
        .cloned()
        .or_else(|| first_set(environment, names).map(SecretString::from));
    Some((*provider, api_key))
}
