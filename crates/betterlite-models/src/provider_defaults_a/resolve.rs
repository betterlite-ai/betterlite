mod tests;

use secrecy::SecretString;

use crate::identity::provider_id::ProviderId;
use crate::json_provider_registry::{JSONProviderRegistry, ProviderDescriptor};
use crate::provider_defaults_a::azure;
use crate::provider_defaults_a::environment::{EnvironmentLookup, first_set};
use crate::provider_defaults_a::error::ProviderDefaultsError;
use crate::provider_defaults_a::model::{
    DefaultsProvider, ProviderDefaultsRequest, ResolvedProvider,
};
use crate::provider_defaults_a::policies::ProviderPolicies;
use crate::provider_defaults_a::special::{self, SPECIAL_TABLE, SpecialDefaults};
use crate::provider_defaults_a::table::{ANYSCALE, SIMPLE_TABLE, SimpleDefaults, simple_defaults};

pub fn resolve_provider_defaults<E>(
    request: ProviderDefaultsRequest,
    env: &E,
    registry: &JSONProviderRegistry,
    policies: &ProviderPolicies,
) -> Result<ResolvedProvider, ProviderDefaultsError>
where
    E: EnvironmentLookup,
{
    let provider = request.provider;
    if let DefaultsProvider::Known(id) = provider
        && let Some(descriptor) = registry.providers.get(&id)
    {
        return Ok(registry_defaults(provider, request, descriptor, env));
    }
    match provider {
        DefaultsProvider::Anyscale => {
            let api_key = request
                .api_key
                .as_ref()
                .map(secrecy::ExposeSecret::expose_secret);
            let (base, key) = simple_defaults(&ANYSCALE, request.api_base.as_deref(), api_key, env);
            Ok(ResolvedProvider {
                provider,
                model: request.model,
                api_base: base,
                api_key: key.map(SecretString::from),
            })
        }
        DefaultsProvider::Known(id) => resolve_known(id, request, env, policies),
    }
}

fn resolve_known<E>(
    id: ProviderId,
    request: ProviderDefaultsRequest,
    env: &E,
    policies: &ProviderPolicies,
) -> Result<ResolvedProvider, ProviderDefaultsError>
where
    E: EnvironmentLookup,
{
    if let Some(defaults) = special_defaults(id) {
        return resolve_special(id, request, defaults, env, policies);
    }
    let Some(defaults) = simple_defaults_for(id) else {
        return Err(ProviderDefaultsError::UnknownProvider { provider: id });
    };
    let api_key = request
        .api_key
        .as_ref()
        .map(secrecy::ExposeSecret::expose_secret);
    let (base, key) = simple_defaults(defaults, request.api_base.as_deref(), api_key, env);
    Ok(ResolvedProvider {
        provider: DefaultsProvider::Known(id),
        model: request.model,
        api_base: base,
        api_key: key.map(SecretString::from),
    })
}

fn special_defaults(id: ProviderId) -> Option<&'static SpecialDefaults> {
    SPECIAL_TABLE
        .iter()
        .find_map(|(key, entry)| (*key == id).then_some(entry))
}

fn simple_defaults_for(id: ProviderId) -> Option<&'static SimpleDefaults> {
    SIMPLE_TABLE
        .iter()
        .find_map(|(key, entry)| (*key == id).then_some(entry))
}

fn resolve_special<E>(
    id: ProviderId,
    request: ProviderDefaultsRequest,
    defaults: &SpecialDefaults,
    env: &E,
    policies: &ProviderPolicies,
) -> Result<ResolvedProvider, ProviderDefaultsError>
where
    E: EnvironmentLookup,
{
    match defaults {
        SpecialDefaults::PassThrough => Ok(ResolvedProvider {
            provider: DefaultsProvider::Known(id),
            model: request.model,
            api_base: request.api_base,
            api_key: request.api_key,
        }),
        SpecialDefaults::Guarded(guarded) => special::guarded(id, request, guarded, env),
        SpecialDefaults::Ai21(ai21) => Ok(special::ai21(id, request, ai21, &policies.models, env)),
        SpecialDefaults::AzureAi => azure::azure_ai(id, request, &policies.models, env),
        SpecialDefaults::Mantle => special::mantle(id, policies.mantle.as_ref()),
    }
}

// The JSON registry's generated config class resolves base and key from its own entry before any hardcoded branch.
fn registry_defaults<E>(
    provider: DefaultsProvider,
    request: ProviderDefaultsRequest,
    descriptor: &ProviderDescriptor,
    env: &E,
) -> ResolvedProvider
where
    E: EnvironmentLookup,
{
    let api_key = request
        .api_key
        .as_ref()
        .map(secrecy::ExposeSecret::expose_secret);
    let base = request
        .api_base
        .clone()
        .or_else(|| {
            let name = descriptor.config.api_base_env.as_deref()?;
            first_set(env, &[name])
        })
        .or_else(|| Some(descriptor.config.base_url.clone()));
    let key = api_key
        .map(ToOwned::to_owned)
        .or_else(|| first_set(env, &[descriptor.config.api_key_env.as_str()]));
    ResolvedProvider {
        provider,
        model: request.model,
        api_base: base,
        api_key: key.map(SecretString::from),
    }
}
