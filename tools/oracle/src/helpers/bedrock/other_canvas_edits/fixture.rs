use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::catalog::{CatalogError, ModelCatalog, UnknownFields};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;

// The typed catalog stays empty: eligibility reads the raw model_cost map the case supplies.
pub(super) struct Catalog {
    typed: ModelCatalog,
    known: BTreeSet<String>,
    custom: BTreeSet<String>,
    environment: BTreeMap<String, String>,
}

impl Catalog {
    pub(super) fn new() -> Result<Self, CatalogError> {
        Ok(Self {
            typed: ModelCatalog::parse(b"{}", UnknownFields::Warn)?,
            known: BTreeSet::new(),
            custom: BTreeSet::new(),
            environment: BTreeMap::new(),
        })
    }

    pub(super) fn context(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.typed,
            known_model_names: &self.known,
            custom_providers: &self.custom,
            environment: &self.environment,
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }
}
