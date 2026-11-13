use std::collections::{BTreeMap, BTreeSet};

use crate::cost_service::{CostService, CostServiceError, Policies, name_normalizers};
use crate::state::shared::Shared;
use betterlite_config::types::Config;
use betterlite_core::cost::completion::lookup::LookupSettings;
use betterlite_models::catalog::{BundledCatalog, CatalogError};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_provider::pricing::grounding::ProviderFormulas;

#[derive(Debug, thiserror::Error)]
pub enum SlotError {
    #[error("the bundled model cost map is unavailable: {0}")]
    Bundled(#[from] CatalogError),
    #[error("the cost service could not be built: {0}")]
    Service(#[from] CostServiceError),
}

// The shared cost resource, built once from the bundled price map, the configured model names and the cost adjustments.
pub struct Slot {
    pub service: Result<CostService<ProviderFormulas>, SlotError>,
}

impl Slot {
    #[must_use]
    pub fn new(config: &Config, _shared: &Shared) -> Self {
        Self {
            service: build(config),
        }
    }
}

fn build(config: &Config) -> Result<CostService<ProviderFormulas>, SlotError> {
    let bytes = (BundledCatalog::default().asset)()?;
    let known_model_names: BTreeSet<String> = config
        .model_list
        .iter()
        .map(|deployment| deployment.model_name.clone())
        .collect();
    let settings = LookupSettings {
        known_model_names,
        custom_providers: BTreeSet::new(),
        environment: BTreeMap::new(),
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
        normalizers: name_normalizers(),
    };
    Ok(CostService::new(
        &bytes,
        settings,
        &config.sections.cost_adjustments,
        Policies::native(),
    )?)
}
