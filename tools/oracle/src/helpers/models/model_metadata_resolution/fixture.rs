use std::collections::{BTreeMap, BTreeSet};
use std::io;

use betterlite_bedrock::mantle_inputs::region::split_region_prefix;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_models::catalog::{CatalogError, ModelCatalog, UnknownFields};
use betterlite_models::catalog_generalizations::CatalogGeneralizations;
use betterlite_models::error::ModelsError;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::model_metadata_resolution::names::{
    ModelCostMap, ModelCostMapError, NameNormalizers,
};
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use serde_json::value::{RawValue, to_raw_value};

use super::input::Fixture;

pub(crate) const NORMALIZERS: NameNormalizers = NameNormalizers {
    bedrock_base_model: get_bedrock_base_model,
    bedrock_routing_prefix: strip_bedrock_routing_prefix,
    mantle_region_split: split_region_prefix,
};

#[derive(Debug, thiserror::Error)]
pub(crate) enum FixtureError {
    #[error("model cost map: {source}")]
    ModelCost {
        #[source]
        source: ModelCostMapError,
    },
    #[error("model cost map JSON: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("model catalog: {source}")]
    Catalog {
        #[source]
        source: CatalogError,
    },
    #[error("generalization rules: {source}")]
    Generalizations {
        #[source]
        source: ModelsError,
    },
    #[error("oracle runtime: {source}")]
    Runtime {
        #[source]
        source: io::Error,
    },
}

pub(crate) struct FixtureContext {
    pub(crate) map: ModelCostMap,
    pub(crate) catalog: ModelCatalog,
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) known_model_names: BTreeSet<String>,
    pub(crate) custom_providers: BTreeSet<String>,
    pub(crate) rules: Vec<Box<RawValue>>,
}

impl FixtureContext {
    pub(crate) fn load(fixture: &Fixture) -> Result<Self, FixtureError> {
        let entries = serde_json::to_vec(&fixture.model_cost)
            .map_err(|source| FixtureError::Json { source })?;
        let map =
            ModelCostMap::parse(&entries).map_err(|source| FixtureError::ModelCost { source })?;
        let catalog = typed_catalog(fixture)?;
        Ok(Self {
            map,
            catalog,
            environment: fixture.environment.clone(),
            known_model_names: fixture.known_model_names.iter().cloned().collect(),
            custom_providers: fixture.custom_providers.iter().cloned().collect(),
            rules: fixture.fallback_generalizations.clone().unwrap_or_default(),
        })
    }

    pub(crate) fn resolution(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.known_model_names,
            custom_providers: &self.custom_providers,
            environment: &self.environment,
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }

    pub(crate) fn generalizations(&self) -> Result<CatalogGeneralizations, FixtureError> {
        let (installed, warnings) = CatalogGeneralizations::install(self.rules.clone());
        let Some(first) = warnings.into_iter().next() else {
            return Ok(installed);
        };
        Err(FixtureError::Generalizations { source: first })
    }
}

// The typed catalog is what B174's inference reads; entries it rejects stay only in the cost map.
fn typed_catalog(fixture: &Fixture) -> Result<ModelCatalog, FixtureError> {
    let mut typed: BTreeMap<String, Box<RawValue>> = BTreeMap::new();
    for (key, entry) in &fixture.model_cost {
        let single = BTreeMap::from([(key.clone(), entry.clone())]);
        let bytes = serde_json::to_vec(&single).map_err(|source| FixtureError::Json { source })?;
        if ModelCatalog::parse(&bytes, UnknownFields::Warn).is_ok() {
            typed.insert(key.clone(), entry.clone());
        }
    }
    if let Some(rules) = &fixture.fallback_generalizations {
        // A case input carries the extracted list; the shipped map keeps it under the block's "rules" key.
        let block = BTreeMap::from([("rules", rules.as_slice())]);
        typed.insert(
            "fallback_generalizations".to_owned(),
            to_raw_value(&block).map_err(|source| FixtureError::Json { source })?,
        );
    }
    let bytes = serde_json::to_vec(&typed).map_err(|source| FixtureError::Json { source })?;
    ModelCatalog::parse(&bytes, UnknownFields::Warn)
        .map_err(|source| FixtureError::Catalog { source })
}
