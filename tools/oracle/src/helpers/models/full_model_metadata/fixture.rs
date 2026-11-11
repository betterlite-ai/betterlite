use std::collections::{BTreeMap, BTreeSet};
use std::io;

use betterlite_bedrock::mantle_inputs::region::split_region_prefix;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_models::catalog::{CatalogError, ModelCatalog, UnknownFields};
use betterlite_models::catalog_generalizations::CatalogGeneralizations;
use betterlite_models::entry::ModelFields;
use betterlite_models::error::ModelsError;
use betterlite_models::fireworks_catalog_metadata::Catalog as FireworksCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::model_metadata_resolution::MetadataEnvironment;
use betterlite_models::model_metadata_resolution::names::{
    ModelCostMap, ModelCostMapError, NameNormalizers,
};
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use serde_json::Value;
use serde_json::value::{RawValue, to_raw_value};

use super::input::FixtureInput;

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
    pub(crate) fireworks: FireworksCatalog,
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) known_model_names: BTreeSet<String>,
    pub(crate) custom_providers: BTreeSet<String>,
    pub(crate) rules: Vec<Box<RawValue>>,
}

impl FixtureContext {
    pub(crate) fn load(fixture: &FixtureInput) -> Result<Self, FixtureError> {
        let entries = serde_json::to_vec(&fixture.model_cost)
            .map_err(|source| FixtureError::Json { source })?;
        let map =
            ModelCostMap::parse(&entries).map_err(|source| FixtureError::ModelCost { source })?;
        let catalog = typed_catalog(fixture)?;
        let fireworks = fireworks_catalog(&map);
        let rules = fixture
            .fallback_generalizations
            .iter()
            .flatten()
            .map(|rule| to_raw_value(rule).map_err(|source| FixtureError::Json { source }))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            map,
            catalog,
            fireworks,
            environment: BTreeMap::new(),
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            rules,
        })
    }

    pub(crate) fn replace(&mut self, key: &str, entry: &Value) -> Result<(), FixtureError> {
        let raw = to_raw_value(entry).map_err(|source| FixtureError::Json { source })?;
        self.map.entries.insert(key.to_owned(), raw);
        Ok(())
    }

    pub(crate) fn harness(&self) -> Result<Harness<'_>, FixtureError> {
        let (generalizations, warnings) = CatalogGeneralizations::install(self.rules.clone());
        let Some(first) = warnings.into_iter().next() else {
            let resolution = ResolutionContext {
                catalog: &self.catalog,
                known_model_names: &self.known_model_names,
                custom_providers: &self.custom_providers,
                environment: &self.environment,
                registry: JSONProviderRegistry::load(),
                policies: ProviderPolicies::default(),
                globals: GlobalCredentials::default(),
            };
            return Ok(Harness {
                fixture: self,
                resolution,
                generalizations,
            });
        };
        Err(FixtureError::Generalizations { source: first })
    }
}

pub(crate) struct Harness<'fixture> {
    pub(crate) fixture: &'fixture FixtureContext,
    pub(crate) resolution: ResolutionContext<'fixture>,
    pub(crate) generalizations: CatalogGeneralizations,
}

impl Harness<'_> {
    pub(crate) const fn environment(&self) -> MetadataEnvironment<'_> {
        MetadataEnvironment {
            model_cost: &self.fixture.map.entries,
            vertex: &self.fixture.map.vertex,
            generalizations: &self.generalizations,
            resolution: &self.resolution,
            normalizers: NORMALIZERS,
        }
    }
}

// The typed catalog is what the provider inference reads; entries it rejects stay only in the cost map.
fn typed_catalog(fixture: &FixtureInput) -> Result<ModelCatalog, FixtureError> {
    let mut typed: BTreeMap<String, Value> = BTreeMap::new();
    for (key, entry) in &fixture.model_cost {
        typed.insert(key.clone(), entry.clone());
    }
    if let Some(rules) = &fixture.fallback_generalizations {
        // a case input carries the extracted list; the shipped map keeps it under the block's "rules" key
        let block = BTreeMap::from([("rules", rules.as_slice())]);
        typed.insert(
            "fallback_generalizations".to_owned(),
            serde_json::to_value(block).map_err(|source| FixtureError::Json { source })?,
        );
    }
    let bytes = serde_json::to_vec(&typed).map_err(|source| FixtureError::Json { source })?;
    ModelCatalog::parse(&bytes, UnknownFields::Warn)
        .map_err(|source| FixtureError::Catalog { source })
}

fn fireworks_catalog(map: &ModelCostMap) -> FireworksCatalog {
    map.entries
        .iter()
        .filter_map(|(key, value)| {
            let fields: ModelFields = serde_json::from_str(value.get()).ok()?;
            Some((key.clone(), fields))
        })
        .collect()
}
