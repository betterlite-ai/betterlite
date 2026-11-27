use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_logging::prometheus::failure::RequestedModelCatalog;
use betterlite_logging::prometheus::failure::provider::{
    ProviderRequestData, api_provider_from_request_data,
};
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;

// the model cost catalog and registry get_llm_provider reads when only a model name names the provider
pub(crate) struct ProviderResolution {
    catalog: Arc<ModelCatalog>,
    known_model_names: BTreeSet<String>,
    custom_providers: BTreeSet<String>,
    environment: BTreeMap<String, String>,
    registry: JSONProviderRegistry,
    policies: ProviderPolicies,
    globals: GlobalCredentials,
}

impl ProviderResolution {
    pub(crate) fn bundled() -> Self {
        Self {
            catalog: BundledCatalog::default().get(),
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            environment: BTreeMap::new(),
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }

    pub(crate) fn api_provider(&self, request: &ProviderRequestData<'_>) -> Option<String> {
        api_provider_from_request_data(request, &self.context())
    }

    fn context(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: self.catalog.as_ref(),
            known_model_names: &self.known_model_names,
            custom_providers: &self.custom_providers,
            environment: &self.environment,
            registry: self.registry.clone(),
            policies: self.policies.clone(),
            globals: self.globals.clone(),
        }
    }
}

// the router port is a trait object, and a borrowed trait object cannot upcast, so this adapter names the trait again
pub(crate) struct CatalogAdapter(pub(crate) Arc<dyn RequestedModelCatalog + Send + Sync>);

impl RequestedModelCatalog for CatalogAdapter {
    fn is_recognized_model(&self, model: &str) -> bool {
        self.0.is_recognized_model(model)
    }

    fn is_team_public_model(&self, model: &str) -> bool {
        self.0.is_team_public_model(model)
    }

    fn matches_pattern_route(&self, model: &str) -> bool {
        self.0.matches_pattern_route(model)
    }
}
