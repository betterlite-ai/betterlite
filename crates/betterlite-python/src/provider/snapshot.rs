use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_models::catalog::ModelCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;

use crate::provider::convert::parse;
use crate::provider::error::ProviderError;
use crate::provider::settings::ProviderSettings;

// Every outside value a lookup reads, taken once when Python builds the native object.
#[derive(Debug)]
pub(crate) struct ProviderSnapshot {
    pub(crate) catalog: Arc<ModelCatalog>,
    pub(crate) known_model_names: BTreeSet<String>,
    pub(crate) custom_providers: BTreeSet<String>,
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) registry: JSONProviderRegistry,
    pub(crate) globals: GlobalCredentials,
}

impl ProviderSnapshot {
    pub(crate) fn build_with(
        text: &str,
        environment: BTreeMap<String, String>,
        catalog: Arc<ModelCatalog>,
    ) -> Result<Self, ProviderError> {
        let settings: ProviderSettings = parse(text)?;
        Ok(Self {
            catalog,
            known_model_names: settings.model_list_set.into_iter().collect(),
            custom_providers: settings
                .custom_provider_map
                .into_iter()
                .map(|entry| entry.provider)
                .collect(),
            environment,
            registry: JSONProviderRegistry::load(),
            globals: GlobalCredentials {
                api_key: settings.api_key,
                api_base: settings.api_base,
                xai_key: settings.xai_key,
                inception_key: settings.inception_key,
                lemonade_key: settings.lemonade_key,
            },
        })
    }

    pub(crate) fn context(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.known_model_names,
            custom_providers: &self.custom_providers,
            environment: &self.environment,
            registry: self.registry.clone(),
            policies: ProviderPolicies::default(),
            globals: self.globals.clone(),
        }
    }
}
