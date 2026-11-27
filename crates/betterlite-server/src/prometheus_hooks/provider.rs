use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_logging::prometheus::failure::RequestedModelCatalog;
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;

// the model cost catalog and registry get_llm_provider reads when only a model name names the provider
pub(crate) struct ProviderResolution {
    pub(crate) catalog: Arc<ModelCatalog>,
    pub(crate) known_model_names: BTreeSet<String>,
    pub(crate) custom_providers: BTreeSet<String>,
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) registry: JSONProviderRegistry,
    pub(crate) policies: ProviderPolicies,
    pub(crate) globals: GlobalCredentials,
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

// both failure emitters take the catalog as a borrowed trait object, so the adapter coerces once here
pub(crate) fn requested_catalog(
    catalog: Option<&CatalogAdapter>,
) -> Option<&dyn RequestedModelCatalog> {
    let adapter = catalog?;
    let requested: &dyn RequestedModelCatalog = adapter;
    Some(requested)
}
