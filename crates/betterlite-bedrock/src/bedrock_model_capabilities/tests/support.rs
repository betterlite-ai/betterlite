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

use super::ModelInfoLookup;
use super::tool_search::supports_tool_search;
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

pub(super) fn packaged() -> Arc<ModelCatalog> {
    BundledCatalog::default().get()
}

pub(super) fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
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
    let bytes = model_cost.as_bytes();
    let entries = ModelCostMap::parse(bytes).expect("the model cost map parses");
    let mut bundled = BundledCatalog::default();
    let catalog = bundled.get();
    let typed = ModelCatalog::parse(b"{}", UnknownFields::Warn).expect("an empty catalog parses");
    let known = BTreeSet::new();
    let custom = BTreeSet::new();
    let variables = BTreeMap::new();
    let resolution = ResolutionContext {
        catalog: &typed,
        known_model_names: &known,
        custom_providers: &custom,
        environment: &variables,
        registry: JSONProviderRegistry::load(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let dynamic = NoDynamicMetadata;
    let hf = HfConfigClient {
        client: NoHfTransport,
        base_url: HF_CONFIG_BASE_URL.to_owned(),
    };
    let provider_catalog = Catalog::new();
    let patterns = CandidatePatterns::new().expect("the candidate patterns compile");
    let raw_runtime: RawCatalog = serde_json::from_slice(bytes).expect("the raw catalog parses");
    let raw_packaged = RawCatalog::new();
    let catalogs = CatalogSources {
        patterns: &patterns,
        runtime: &raw_runtime,
        packaged: &raw_packaged,
    };
    let mut keys = ModelCostKeys::default();
    let mut environment = CapabilityEnvironment {
        metadata: MetadataEnvironment {
            model_cost: &entries.entries,
            vertex: &entries.vertex,
            generalizations: &catalog.generalizations,
            resolution: &resolution,
            normalizers: NORMALIZERS,
        },
        keys: &mut keys,
        dynamic: &dynamic,
        hf: &hf,
        provider_catalog: &provider_catalog,
        provider_generation: 0,
    };
    let runtime = runtime();
    models
        .iter()
        .map(|model| runtime.block_on(supports_tool_search(*model, &mut environment, &catalogs)))
        .collect()
}
