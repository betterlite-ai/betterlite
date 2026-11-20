use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::cost_service::{CostService, CostServiceError, Policies, name_normalizers};
use crate::savings::{Context, DeploymentPricing};
use crate::state::shared::Shared;
use betterlite_config::types::Config;
use betterlite_core::cost::completion::lookup::{CatalogLookup, LookupSettings};
use betterlite_models::catalog::{BundledCatalog, CatalogError, ModelCatalog};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_provider::pricing::grounding::ProviderFormulas;
use jiff::tz::TimeZoneDatabase;

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
    deployments: DeploymentPricing,
}

impl Slot {
    #[must_use]
    pub fn new(config: &Config, shared: &Shared) -> Self {
        Self {
            service: build(
                config,
                Arc::clone(&shared.bundled_catalog),
                TimeZoneDatabase::from_env(),
            ),
            deployments: DeploymentPricing::default(),
        }
    }

    // The savings side of this resource: its bound public lookup and the deployment overrides.
    #[must_use]
    pub fn savings_context(&self) -> Option<Context<'_>> {
        let service = self.service.as_ref().ok()?;
        Some(Context::new(&service.lookup, &self.deployments))
    }
}

fn build(
    config: &Config,
    catalog: Arc<ModelCatalog>,
    database: TimeZoneDatabase,
) -> Result<CostService<ProviderFormulas>, SlotError> {
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
    let lookup = CatalogLookup::new(&bytes, catalog, settings).map_err(CostServiceError::Lookup)?;
    Ok(CostService::new(
        lookup,
        &config.sections.cost_adjustments,
        Policies::native(),
        database,
    )?)
}
