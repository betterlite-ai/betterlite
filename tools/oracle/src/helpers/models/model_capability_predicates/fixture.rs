use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};

use betterlite_bedrock::mantle_inputs::region::split_region_prefix;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_models::catalog::{CatalogError, ModelCatalog, UnknownFields};
use betterlite_models::catalog_generalizations::CatalogGeneralizations;
use betterlite_models::entry::ModelFields;
use betterlite_models::fetch_policy::response::FetchResponse;
use betterlite_models::fireworks_catalog_metadata::Catalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::lemonade_model_metadata::{ModelInfoClient, ModelInfoRequest};
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::MetadataEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::NoDynamicMetadata;
use betterlite_models::model_metadata_resolution::hf_config::{HF_CONFIG_BASE_URL, HfConfigClient};
use betterlite_models::model_metadata_resolution::names::{
    ModelCostMap, ModelCostMapError, NameNormalizers,
};
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::value::RawValue;

const NORMALIZERS: NameNormalizers = NameNormalizers {
    bedrock_base_model: get_bedrock_base_model,
    bedrock_routing_prefix: strip_bedrock_routing_prefix,
    mantle_region_split: split_region_prefix,
};

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FixtureInput {
    #[serde(default)]
    model_cost: IndexMap<String, Box<RawValue>>,
}

#[derive(Debug, thiserror::Error)]
pub(super) enum FixtureError {
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
    #[error("oracle runtime: {source}")]
    Runtime {
        #[source]
        source: std::io::Error,
    },
}

// The capability cases name no huggingface model, so the transport refuses rather than reaching the network.
pub(super) struct NoHfTransport;

#[derive(Debug, thiserror::Error)]
#[error("the metadata lookup must not fetch a Hugging Face config")]
pub(super) struct NoHfError;

impl ModelInfoClient for NoHfTransport {
    type Error = NoHfError;

    fn get(
        &self,
        _request: &ModelInfoRequest,
    ) -> impl Future<Output = Result<FetchResponse, NoHfError>> + Send {
        ready(Err(NoHfError))
    }
}

pub(super) struct Fixture {
    map: ModelCostMap,
    catalog: ModelCatalog,
    generalizations: CatalogGeneralizations,
    provider_catalog: Catalog,
    hf: HfConfigClient<NoHfTransport>,
    known_model_names: BTreeSet<String>,
    custom_providers: BTreeSet<String>,
    environment: BTreeMap<String, String>,
}

impl Fixture {
    pub(super) fn load(input: &FixtureInput) -> Result<Self, FixtureError> {
        let bytes = serde_json::to_vec(&input.model_cost)
            .map_err(|source| FixtureError::Json { source })?;
        Ok(Self {
            map: ModelCostMap::parse(&bytes)
                .map_err(|source| FixtureError::ModelCost { source })?,
            catalog: typed_catalog(&input.model_cost)?,
            generalizations: CatalogGeneralizations::default(),
            provider_catalog: fireworks_catalog(&input.model_cost),
            hf: HfConfigClient {
                client: NoHfTransport,
                base_url: HF_CONFIG_BASE_URL.to_owned(),
            },
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            environment: BTreeMap::new(),
        })
    }

    pub(super) fn resolution(&self) -> ResolutionContext<'_> {
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

    pub(super) const fn environment<'fixture>(
        &'fixture self,
        resolution: &'fixture ResolutionContext<'fixture>,
        keys: &'fixture mut ModelCostKeys,
        dynamic: &'fixture NoDynamicMetadata,
    ) -> CapabilityEnvironment<'fixture, NoDynamicMetadata, NoHfTransport> {
        CapabilityEnvironment {
            metadata: MetadataEnvironment {
                model_cost: &self.map.entries,
                vertex: &self.map.vertex,
                generalizations: &self.generalizations,
                resolution,
                normalizers: NORMALIZERS,
            },
            keys,
            dynamic,
            hf: &self.hf,
            provider_catalog: &self.provider_catalog,
            provider_generation: 0,
        }
    }
}

// The typed catalog is what inference reads; an entry with a wrongly typed flag stays only in the cost map.
fn typed_catalog(entries: &IndexMap<String, Box<RawValue>>) -> Result<ModelCatalog, FixtureError> {
    let mut typed: IndexMap<String, Box<RawValue>> = IndexMap::new();
    for (key, entry) in entries {
        let single = IndexMap::from([(key.clone(), entry.clone())]);
        let bytes = serde_json::to_vec(&single).map_err(|source| FixtureError::Json { source })?;
        if ModelCatalog::parse(&bytes, UnknownFields::Warn).is_ok() {
            typed.insert(key.clone(), entry.clone());
        }
    }
    let bytes = serde_json::to_vec(&typed).map_err(|source| FixtureError::Json { source })?;
    ModelCatalog::parse(&bytes, UnknownFields::Warn)
        .map_err(|source| FixtureError::Catalog { source })
}

// LiteLLM's fireworks capability lookup reads the bare model key, so the catalog keeps every parseable entry.
fn fireworks_catalog(entries: &IndexMap<String, Box<RawValue>>) -> Catalog {
    let mut catalog = Catalog::new();
    for (key, entry) in entries {
        if let Ok((fields, _warnings)) = ModelFields::parse(entry) {
            catalog.insert(key.clone(), fields);
        }
    }
    catalog
}
