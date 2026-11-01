mod tests;

use secrecy::SecretString;

use crate::identity::provider_id::ProviderId;
use crate::provider_defaults_b::environment::EnvironmentLookup;
use crate::provider_defaults_b::error::ProviderDefaultsError;
use crate::provider_defaults_b::globals::GlobalCredentials;
use crate::provider_defaults_b::model::{ProviderDefaultsRequest, ResolvedProvider};
use crate::provider_defaults_b::special::{
    SPECIAL_TABLE, resolve_mistral, resolve_ragflow, resolve_special,
};
use crate::provider_defaults_b::table::{SIMPLE_TABLE, SimpleDefaults};

fn resolve_simple<E>(
    defaults: &SimpleDefaults,
    api_base: Option<&str>,
    api_key: Option<&str>,
    env: &E,
) -> (Option<String>, Option<String>)
where
    E: EnvironmentLookup,
{
    let base = api_base
        .map(ToOwned::to_owned)
        .or_else(|| defaults.base_env.iter().find_map(|name| env.lookup(name)))
        .or_else(|| defaults.base_default.map(ToOwned::to_owned));
    let key = api_key
        .map(ToOwned::to_owned)
        .or_else(|| defaults.key_env.iter().find_map(|name| env.lookup(name)))
        .or_else(|| defaults.key_default.map(ToOwned::to_owned));
    (base, key)
}

pub fn resolve_provider_defaults<E>(
    request: ProviderDefaultsRequest,
    globals: &GlobalCredentials,
    env: &E,
) -> Result<ResolvedProvider, ProviderDefaultsError>
where
    E: EnvironmentLookup,
{
    let provider = request.provider;
    let base_in = request.api_base.as_deref();
    let key_in = request
        .api_key
        .as_ref()
        .map(secrecy::ExposeSecret::expose_secret);

    if provider == ProviderId::Ragflow {
        let (model, resolved_base, resolved_key) =
            resolve_ragflow(&request.model, base_in, key_in, globals, env)?;
        return Ok(ResolvedProvider {
            model,
            api_base: resolved_base,
            api_key: resolved_key.map(SecretString::from),
        });
    }

    let special = SPECIAL_TABLE
        .iter()
        .find_map(|(id, defaults)| (*id == provider).then_some(defaults));

    let (resolved_base, resolved_key) = if provider == ProviderId::Mistral {
        resolve_mistral(base_in, key_in, env)
    } else if let Some(defaults) = special {
        resolve_special(base_in, key_in, defaults, globals, env)
    } else {
        let defaults = SIMPLE_TABLE
            .iter()
            .find_map(|(id, defaults)| (*id == provider).then_some(defaults))
            .ok_or(ProviderDefaultsError::UnknownProvider { provider })?;
        resolve_simple(defaults, base_in, key_in, env)
    };

    Ok(ResolvedProvider {
        model: request.model,
        api_base: resolved_base,
        api_key: resolved_key.map(SecretString::from),
    })
}
