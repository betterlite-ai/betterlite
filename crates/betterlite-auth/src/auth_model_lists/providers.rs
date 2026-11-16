use betterlite_models::identity::provider_id::ProviderId;
use strum::VariantArray as _;

use crate::auth_model_lists::parameters::ModelParameters;

// providers.json registers these slugs, but litellm.types.utils.LlmProviders does not, so provider_list lacks them.
const REGISTRY_ONLY_PROVIDERS: [ProviderId; 8] = [
    ProviderId::Abliteration,
    ProviderId::Aihubmix,
    ProviderId::Crusoe,
    ProviderId::Empiriolabs,
    ProviderId::Gmi,
    ProviderId::Llamagate,
    ProviderId::Sarvam,
    ProviderId::Veniceai,
];

// B2328's get_valid_models, injected so a listing never reaches a provider endpoint from here.
pub trait ProviderModels: Send + Sync {
    fn valid_models(
        &self,
        provider: Option<&str>,
        parameters: Option<&ModelParameters>,
    ) -> Option<Vec<String>>;
}

// _check_wildcard_routing: any asterisk anywhere makes a model a wildcard.
#[must_use]
pub fn is_wildcard(model: &str) -> bool {
    model.contains('*')
}

// get_provider_models: provider "*" asks every configured provider, an unlisted provider has no models.
#[must_use]
pub fn get_provider_models(
    provider: &str,
    parameters: Option<&ModelParameters>,
    providers: &dyn ProviderModels,
) -> Option<Vec<String>> {
    let requested = if provider == "*" { None } else { Some(provider) };
    providers.valid_models(requested, parameters)
}

// `{provider.value for provider in LlmProviders}`, which the wildcard prefix replacement compares against.
#[must_use]
pub fn is_declared_provider(segment: &str) -> bool {
    segment
        .parse::<ProviderId>()
        .is_ok_and(|provider| !REGISTRY_ONLY_PROVIDERS.contains(&provider))
}

// litellm.provider_list: every declared provider, ordered as the enum declares them.
#[must_use]
pub fn provider_list() -> Vec<ProviderId> {
    ProviderId::VARIANTS
        .iter()
        .copied()
        .filter(|provider| !REGISTRY_ONLY_PROVIDERS.contains(provider))
        .collect()
}
