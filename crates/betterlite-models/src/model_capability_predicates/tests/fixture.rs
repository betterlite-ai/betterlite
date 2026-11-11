use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};

use serde_json::value::RawValue;

use super::super::CapabilityEnvironment;
use crate::catalog::{ModelCatalog, UnknownFields};
use crate::catalog_generalizations::CatalogGeneralizations;
use crate::entry::ModelFields;
use crate::fetch_policy::response::FetchResponse;
use crate::fireworks_catalog_metadata::Catalog;
use crate::json_provider_registry::JSONProviderRegistry;
use crate::lemonade_model_metadata::{ModelInfoClient, ModelInfoRequest};
use crate::model_catalog_keys::ModelCostKeys;
use crate::model_metadata_resolution::MetadataEnvironment;
use crate::model_metadata_resolution::dynamic::NoDynamicMetadata;
use crate::model_metadata_resolution::hf_config::{HF_CONFIG_BASE_URL, HfConfigClient};
use crate::model_metadata_resolution::names::{ModelCostMap, NameNormalizers};
use crate::provider_defaults_a::policies::ProviderPolicies;
use crate::provider_defaults_b::globals::GlobalCredentials;
use crate::resolution::ResolutionContext;

const NORMALIZERS: NameNormalizers = NameNormalizers {
    bedrock_base_model: test_bedrock_base_model,
    bedrock_routing_prefix: test_bedrock_routing_prefix,
    mantle_region_split: test_mantle_region_split,
};

fn test_bedrock_base_model(model: &str) -> String {
    let stripped = model.strip_prefix("converse/").unwrap_or(model);
    let regional = stripped.strip_prefix("us.").unwrap_or(stripped);
    regional.to_owned()
}

fn test_bedrock_routing_prefix(model: &str) -> String {
    model
        .strip_prefix("converse/")
        .map_or_else(|| model.to_owned(), str::to_owned)
}

fn test_mantle_region_split(model: &str) -> (Option<&str>, &str) {
    match model.split_once('/') {
        Some((head, rest)) if head == "us-east-1" => (Some(head), rest),
        _ => (None, model),
    }
}

// The metadata helper only reaches the transport for a huggingface provider, which no capability case uses.
pub(crate) struct NoHfTransport;

#[derive(Debug, thiserror::Error)]
#[error("the metadata lookup must not fetch a Hugging Face config")]
pub(crate) struct NoHfError;

impl ModelInfoClient for NoHfTransport {
    type Error = NoHfError;

    fn get(
        &self,
        _request: &ModelInfoRequest,
    ) -> impl Future<Output = Result<FetchResponse, NoHfError>> + Send {
        ready(Err(NoHfError))
    }
}

pub(crate) struct Fixture {
    pub(crate) map: ModelCostMap,
    pub(crate) catalog: ModelCatalog,
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) known_model_names: BTreeSet<String>,
    pub(crate) custom_providers: BTreeSet<String>,
    pub(crate) generalizations: CatalogGeneralizations,
    pub(crate) provider_catalog: Catalog,
    pub(crate) hf: HfConfigClient<NoHfTransport>,
}

impl Fixture {
    pub(crate) fn new(source: &str) -> Self {
        Self {
            map: ModelCostMap::parse(source.as_bytes()).expect("model cost map"),
            catalog: typed_catalog(source),
            environment: BTreeMap::new(),
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            generalizations: CatalogGeneralizations::default(),
            provider_catalog: fireworks_catalog(source),
            hf: HfConfigClient {
                client: NoHfTransport,
                base_url: HF_CONFIG_BASE_URL.to_owned(),
            },
        }
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

    pub(crate) fn environment<'fixture>(
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

// One cost map and its caches, so every query in one case reads the map the way LiteLLM's module globals do.
pub(crate) struct Session<'fixture> {
    fixture: &'fixture Fixture,
    resolution: ResolutionContext<'fixture>,
    keys: ModelCostKeys,
    dynamic: NoDynamicMetadata,
}

impl<'fixture> Session<'fixture> {
    pub(crate) fn new(fixture: &'fixture Fixture) -> Self {
        let resolution = fixture.resolution();
        Self {
            fixture,
            resolution,
            keys: ModelCostKeys::default(),
            dynamic: NoDynamicMetadata,
        }
    }

    pub(crate) fn environment(
        &mut self,
    ) -> CapabilityEnvironment<'_, NoDynamicMetadata, NoHfTransport> {
        self.fixture
            .environment(&self.resolution, &mut self.keys, &self.dynamic)
    }
}

// The typed catalog is what inference reads; an entry with a wrongly typed flag stays only in the cost map.
fn typed_catalog(source: &str) -> ModelCatalog {
    let entries: BTreeMap<String, Box<RawValue>> =
        serde_json::from_str(source).expect("model cost map");
    let mut typed: BTreeMap<String, Box<RawValue>> = BTreeMap::new();
    for (key, entry) in entries {
        let single = BTreeMap::from([(key.clone(), entry.clone())]);
        let bytes = serde_json::to_vec(&single).expect("model cost entry");
        if ModelCatalog::parse(&bytes, UnknownFields::Warn).is_ok() {
            typed.insert(key, entry);
        }
    }
    let bytes = serde_json::to_vec(&typed).expect("typed catalog");
    ModelCatalog::parse(&bytes, UnknownFields::Warn).expect("typed catalog")
}

// LiteLLM's fireworks capability lookup reads the bare model key, so the catalog keeps every parseable entry.
fn fireworks_catalog(source: &str) -> Catalog {
    let entries: BTreeMap<String, Box<RawValue>> =
        serde_json::from_str(source).expect("model cost map");
    entries
        .into_iter()
        .filter_map(|(key, entry)| {
            let fields = ModelFields::parse(&entry).ok()?.0;
            Some((key, fields))
        })
        .collect()
}
