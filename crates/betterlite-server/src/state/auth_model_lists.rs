use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::Arc;

use betterlite_auth::auth_model_lists::complete_list::ListingRequest;
use betterlite_auth::auth_model_lists::listing::ListingService;
use betterlite_auth::auth_model_lists::parameters::ModelParameters;
use betterlite_auth::auth_model_lists::providers::{ProviderModels, provider_list};
use betterlite_auth::auth_model_lists::wildcard::WildcardError;
use betterlite_config::types::Config;
use betterlite_models::available_models::cache::AvailableModelsCache;
use betterlite_models::available_models::environment::EnvironmentSnapshot;
use betterlite_models::available_models::listing::{ListingCredentials, ModelListing};
use betterlite_models::available_models::parameters::LiteLlmParams;
use betterlite_models::available_models::{EnumerationRequest, get_valid_models};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::provider_metadata_factory::selection::ProviderModelMetadata;
use betterlite_models::provider_model_sets::ProviderModelSets;
use secrecy::{ExposeSecret as _, SecretString};

use crate::state::shared::Shared;
pub struct Slot {
    pub(crate) service: ListingService,
    providers: CatalogModels,
}
impl Slot {
    #[must_use]
    pub fn new(config: &Config, shared: &Shared) -> Self {
        Self {
            service: ListingService::from_config(config),
            providers: CatalogModels::from_config(config, &shared.bundled_catalog),
        }
    }

    pub fn authorized_models(
        &self,
        request: &ListingRequest<'_>,
    ) -> Result<Vec<String>, WildcardError> {
        self.service
            .authorized_models(request, &self.providers, None)
    }
}
struct CatalogModels {
    models_by_provider: BTreeMap<ProviderId, Vec<String>>,
    provider_list: Vec<ProviderId>,
    environment: EnvironmentSnapshot,
    check_provider_endpoint: bool,
}
impl CatalogModels {
    fn from_config(config: &Config, catalog: &ModelCatalog) -> Self {
        let mut sets = ProviderModelSets::default();
        sets.add_known_models(catalog);
        Self {
            models_by_provider: sets
                .models_by_provider
                .iter()
                .filter_map(|(name, models)| {
                    let provider = name.parse::<ProviderId>().ok()?;
                    Some((provider, models.iter().cloned().collect()))
                })
                .collect(),
            provider_list: provider_list(),
            environment: EnvironmentSnapshot::from(&environment_variables(config)),
            check_provider_endpoint: config.sections.available_models.check_provider_endpoint,
        }
    }
}
impl ProviderModels for CatalogModels {
    fn valid_models(
        &self,
        provider: Option<&str>,
        parameters: Option<&ModelParameters>,
    ) -> Option<Vec<String>> {
        if let Some(requested) = provider
            && !self
                .models_by_provider
                .keys()
                .any(|known| known.to_string() == requested)
        {
            return None;
        }
        let request = EnumerationRequest {
            check_provider_endpoint: self.check_provider_endpoint,
            global_check_provider_endpoint: false,
            custom_llm_provider: provider,
            parameters: parameters.map_or_else(LiteLlmParams::default, available_parameters),
            api_key: None,
            api_base: None,
            provider_list: &self.provider_list,
            models_by_provider: &self.models_by_provider,
            environment: &self.environment,
        };
        // the cache lives for one enumeration, so the clock never has to advance for an entry to live
        let mut cache = AvailableModelsCache::with_clock(Arc::new(|| 0.0_f64));
        let enumeration = get_valid_models(&request, &mut cache, &ProviderEndpointListing);
        Some(enumeration.models)
    }
}
fn available_parameters(parameters: &ModelParameters) -> LiteLlmParams {
    LiteLlmParams {
        model: parameters.model().to_owned(),
        api_key: parameters.text("api_key").map(SecretString::from),
        api_base: parameters.text("api_base").map(str::to_owned),
        api_version: parameters.text("api_version").map(str::to_owned),
        custom_llm_provider: parameters
            .text("custom_llm_provider")
            .and_then(|provider| provider.parse().ok()),
    }
}
// `environment_variables:` is the proxy's own env source; an unset reference keeps the name only.
fn environment_variables(config: &Config) -> BTreeMap<String, String> {
    config
        .environment_variables
        .iter()
        .map(|(name, value)| {
            let text = value
                .as_ref()
                .map_or_else(String::new, |secret| secret.expose_secret().to_owned());
            (name.clone(), text)
        })
        .collect()
}
struct ProviderEndpointListing;
impl ModelListing for ProviderEndpointListing {
    // no provider endpoint catalogue is built, so an endpoint check lists no models
    type Error = Infallible;

    fn get_models(
        &self,
        _adapter: &ProviderModelMetadata,
        _credentials: ListingCredentials<'_>,
    ) -> Result<Vec<String>, Infallible> {
        Ok(Vec::new())
    }
}
