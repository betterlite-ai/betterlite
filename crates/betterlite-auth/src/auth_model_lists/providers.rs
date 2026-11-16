use betterlite_models::identity::provider_id::ProviderId;
use strum::VariantArray as _;

use crate::auth_model_lists::parameters::ModelParameters;
// providers.json registers these slugs, but litellm.types.utils.LlmProviders does not, so provider_list lacks them.
#[rustfmt::skip]
const REGISTRY_ONLY_PROVIDERS: [ProviderId; 8] = [
    ProviderId::Abliteration, ProviderId::Aihubmix, ProviderId::Crusoe, ProviderId::Empiriolabs,
    ProviderId::Gmi, ProviderId::Llamagate, ProviderId::Sarvam, ProviderId::Veniceai,
];
pub trait ProviderModels: Send + Sync {
    fn valid_models(
        &self,
        provider: Option<&str>,
        parameters: Option<&ModelParameters>,
    ) -> Option<Vec<String>>;
}
#[must_use]
// ast-grep-ignore: no-one-line-functions
pub fn is_wildcard(model: &str) -> bool {
    model.contains('*')
}
#[must_use]
pub fn get_provider_models(
    provider: &str,
    parameters: Option<&ModelParameters>,
    providers: &dyn ProviderModels,
) -> Option<Vec<String>> {
    match provider {
        "*" => providers.valid_models(None, parameters),
        _ => providers.valid_models(Some(provider), parameters),
    }
}
// `{provider.value for provider in LlmProviders}`, which the wildcard prefix replacement compares against.
#[must_use]
pub fn is_declared_provider(segment: &str) -> bool {
    segment
        .parse::<ProviderId>()
        .is_ok_and(|provider| !REGISTRY_ONLY_PROVIDERS.contains(&provider))
}
#[must_use]
pub fn provider_list() -> Vec<ProviderId> {
    ProviderId::VARIANTS
        .iter()
        .copied()
        .filter(|provider| !REGISTRY_ONLY_PROVIDERS.contains(provider))
        .collect()
}
