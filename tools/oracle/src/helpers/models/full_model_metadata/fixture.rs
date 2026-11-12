use std::collections::{BTreeMap, BTreeSet};

use betterlite_bedrock::mantle_inputs::region::split_region_prefix;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_models::catalog::{CatalogError, ModelCatalog, UnknownFields};
use betterlite_models::catalog_generalizations::CatalogGeneralizations;
use betterlite_models::entry::ModelFields;
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
use serde_json::value::to_raw_value;

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
}

pub(crate) struct FixtureContext {
    pub(crate) map: ModelCostMap,
    pub(crate) catalog: ModelCatalog,
    pub(crate) fireworks: FireworksCatalog,
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) known_model_names: BTreeSet<String>,
    pub(crate) custom_providers: BTreeSet<String>,
}

impl FixtureContext {
    pub(crate) fn load(fixture: &FixtureInput) -> Result<Self, FixtureError> {
        let entries = serde_json::to_vec(&fixture.model_cost)
            .map_err(|source| FixtureError::Json { source })?;
        let map =
            ModelCostMap::parse(&entries).map_err(|source| FixtureError::ModelCost { source })?;
        let catalog = ModelCatalog::parse(&entries, UnknownFields::Warn)
            .map_err(|source| FixtureError::Catalog { source })?;
        let fireworks = fireworks_catalog(&map);
        Ok(Self {
            map,
            catalog,
            fireworks,
            environment: BTreeMap::new(),
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
        })
    }

    pub(crate) fn replace(&mut self, key: &str, entry: &Value) -> Result<(), FixtureError> {
        let raw = to_raw_value(entry).map_err(|source| FixtureError::Json { source })?;
        self.map.entries.insert(key.to_owned(), raw);
        Ok(())
    }

    pub(crate) fn harness(&self) -> Harness<'_> {
        let (generalizations, _warnings) = CatalogGeneralizations::install(Vec::new());
        Harness {
            fixture: self,
            resolution: ResolutionContext {
                catalog: &self.catalog,
                known_model_names: &self.known_model_names,
                custom_providers: &self.custom_providers,
                environment: &self.environment,
                registry: JSONProviderRegistry::load(),
                policies: ProviderPolicies::default(),
                globals: GlobalCredentials::default(),
            },
            generalizations,
        }
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

fn fireworks_catalog(map: &ModelCostMap) -> FireworksCatalog {
    map.entries
        .iter()
        .filter_map(|(key, value)| {
            let fields: ModelFields = serde_json::from_str(value.get()).ok()?;
            Some((key.clone(), fields))
        })
        .collect()
}
