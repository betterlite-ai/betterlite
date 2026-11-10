use std::collections::BTreeMap;

use secrecy::SecretString;

use crate::identity::endpoint::endpoint_matches_api_base;
use crate::provider_defaults_a::environment::first_set;

use super::{ProviderIdentity, ResolutionContext, ResolutionError, ResolutionRequest};
use table::{BRANCHES, Branch, ENDPOINTS, EndpointProvider, KeySource};

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
    if let Some(branch) = BRANCHES.iter().find(|branch| branch.endpoint == endpoint) {
        return Ok(Some(EndpointMatch {
            provider: identity(branch.provider),
            api_key: key(branch, api_base, request, context.environment),
        }));
    }
    if let Some(descriptor) = context.registry.get_by_base_url(endpoint) {
        let environment = context.environment;
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

const fn identity(provider: EndpointProvider) -> ProviderIdentity {
    match provider {
        EndpointProvider::Anyscale => ProviderIdentity::Anyscale,
        EndpointProvider::Known(known) => ProviderIdentity::Known(known),
    }
}

fn key(
    branch: &Branch,
    api_base: &str,
    request: &ResolutionRequest<'_>,
    environment: &BTreeMap<String, String>,
) -> Option<SecretString> {
    match &branch.keys {
        KeySource::Env(names) => first_set(environment, names).map(SecretString::from),
        KeySource::EnvUnlessGiven(names) => request
            .api_key
            .cloned()
            .or_else(|| first_set(environment, names).map(SecretString::from)),
        KeySource::HttpsOnly(names) => api_base
            .to_lowercase()
            .starts_with("https://")
            .then(|| first_set(environment, names))
            .flatten()
            .map(SecretString::from),
    }
}
