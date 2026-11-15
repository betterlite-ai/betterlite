use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::sync::Arc;

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_context::anthropic_catalog_lookup::RawCatalog;
use betterlite_context::anthropic_catalog_lookup::candidates::CandidatePatterns;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_models::catalog::{BundledCatalog, ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_models::fetch_policy::response::FetchResponse;
use betterlite_models::fireworks_catalog_metadata::Catalog;
use betterlite_models::full_model_metadata::record::ModelInfo;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::lemonade_model_metadata::{ModelInfoClient, ModelInfoRequest};
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::MetadataEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::NoDynamicMetadata;
use betterlite_models::model_metadata_resolution::hf_config::{HF_CONFIG_BASE_URL, HfConfigClient};
use betterlite_models::model_metadata_resolution::names::{ModelCostMap, NameNormalizers};
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use indexmap::IndexMap;
use tokio::runtime::{Builder, Runtime};

use super::super::ModelInfoLookup;
use super::super::tool_search::supports_tool_search;
use crate::mantle_inputs::region::split_region_prefix;
use crate::model_routes::ParamMap;

const NORMALIZERS: NameNormalizers = NameNormalizers {
    bedrock_base_model: get_bedrock_base_model,
    bedrock_routing_prefix: strip_bedrock_routing_prefix,
    mantle_region_split: split_region_prefix,
};

pub(super) fn map(source: &str) -> ParamMap {
    serde_json::from_str(source).expect("fixture map parses")
}

pub(super) fn runtime() -> Runtime {
    Builder::new_current_thread()
        .build()
        .expect("the test runtime builds")
}

#[derive(Debug, thiserror::Error)]
#[error("the model is not mapped")]
pub(super) struct Unmapped;

pub(super) struct Answers(BTreeMap<String, IndexMap<String, Json>>);

impl Answers {
    pub(super) fn new(entries: &[(&str, &str)]) -> Self {
        Self(
            entries
                .iter()
                .map(|(model, fields)| {
                    (
                        (*model).to_owned(),
                        serde_json::from_str(fields).expect("model info parses"),
                    )
                })
                .collect(),
        )
    }
}

impl ModelInfoLookup for Answers {
    type Error = Unmapped;

    fn bedrock_model_info(
        &self,
        model: &str,
    ) -> impl Future<Output = Result<ModelInfo, Unmapped>> + Send {
        let answer = self.0.get(model).cloned();
        ready(answer.map(|fields| ModelInfo { fields }).ok_or(Unmapped))
    }
}

pub(super) struct NoHfTransport;

impl ModelInfoClient for NoHfTransport {
    type Error = Unmapped;

    fn get(
        &self,
        _request: &ModelInfoRequest,
    ) -> impl Future<Output = Result<FetchResponse, Unmapped>> + Send {
        ready(Err(Unmapped))
    }
}

// The model-cost-aware capability chain B688 reads, over the fixture map and the bundled rules.
pub(super) fn tool_search_answers(model_cost: &str, models: &[&str]) -> Vec<bool> {
    let fixture = ToolSearchFixture::new(model_cost);
    ToolSearchSession::new(&fixture).answers(models)
}

struct ToolSearchFixture {
    entries: ModelCostMap,
    catalog: Arc<ModelCatalog>,
    typed: ModelCatalog,
    known_model_names: BTreeSet<String>,
    custom_providers: BTreeSet<String>,
    variables: BTreeMap<String, String>,
    hf: HfConfigClient<NoHfTransport>,
    provider_catalog: Catalog,
    patterns: CandidatePatterns,
    runtime_catalog: RawCatalog,
    packaged_catalog: RawCatalog,
}

impl ToolSearchFixture {
    fn new(model_cost: &str) -> Self {
        let mut bundled = BundledCatalog::default();
        Self {
            entries: ModelCostMap::parse(model_cost.as_bytes()).expect("the model cost map parses"),
            catalog: bundled.get(),
            typed: ModelCatalog::parse(b"{}", UnknownFields::Warn)
                .expect("an empty catalog parses"),
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            variables: BTreeMap::new(),
            hf: HfConfigClient {
                client: NoHfTransport,
                base_url: HF_CONFIG_BASE_URL.to_owned(),
            },
            provider_catalog: Catalog::new(),
            patterns: CandidatePatterns::new().expect("the candidate patterns compile"),
            runtime_catalog: serde_json::from_slice(model_cost.as_bytes())
                .expect("the raw catalog parses"),
            packaged_catalog: RawCatalog::new(),
        }
    }

    const fn catalogs(&self) -> CatalogSources<'_> {
        CatalogSources {
            patterns: &self.patterns,
            runtime: &self.runtime_catalog,
            packaged: &self.packaged_catalog,
        }
    }
}

struct ToolSearchSession<'fixture> {
    fixture: &'fixture ToolSearchFixture,
    resolution: ResolutionContext<'fixture>,
    keys: ModelCostKeys,
    dynamic: NoDynamicMetadata,
}

impl<'fixture> ToolSearchSession<'fixture> {
    fn new(fixture: &'fixture ToolSearchFixture) -> Self {
        let resolution = ResolutionContext {
            catalog: &fixture.typed,
            known_model_names: &fixture.known_model_names,
            custom_providers: &fixture.custom_providers,
            environment: &fixture.variables,
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        };
        Self {
            fixture,
            resolution,
            keys: ModelCostKeys::default(),
            dynamic: NoDynamicMetadata,
        }
    }

    fn answers(&mut self, models: &[&str]) -> Vec<bool> {
        let catalogs = self.fixture.catalogs();
        let mut environment = CapabilityEnvironment {
            metadata: MetadataEnvironment {
                model_cost: &self.fixture.entries.entries,
                vertex: &self.fixture.entries.vertex,
                generalizations: &self.fixture.catalog.generalizations,
                resolution: &self.resolution,
                normalizers: NORMALIZERS,
            },
            keys: &mut self.keys,
            dynamic: &self.dynamic,
            hf: &self.fixture.hf,
            provider_catalog: &self.fixture.provider_catalog,
            provider_generation: 0,
        };
        let runtime = runtime();
        models
            .iter()
            .map(|model| runtime.block_on(supports_tool_search(model, &mut environment, &catalogs)))
            .collect()
    }
}
