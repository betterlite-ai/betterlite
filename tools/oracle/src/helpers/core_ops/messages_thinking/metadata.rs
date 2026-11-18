use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};

use betterlite_bedrock::mantle_inputs::region::split_region_prefix;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::catalog_generalizations::CatalogGeneralizations;
use betterlite_models::fetch_policy::response::FetchResponse;
use betterlite_models::full_model_metadata::record::ModelInfo;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::lemonade_model_metadata::{ModelInfoClient, ModelInfoRequest};
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::dynamic::NoDynamicMetadata;
use betterlite_models::model_metadata_resolution::hf_config::{HF_CONFIG_BASE_URL, HfConfigClient};
use betterlite_models::model_metadata_resolution::names::{ModelCostMap, NameNormalizers};
use betterlite_models::model_metadata_resolution::{
    MetadataEnvironment, MetadataRequest, get_model_info_helper,
};
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use indexmap::IndexMap;
use serde_json::Value;
use tokio::runtime::Runtime;

use super::helper_error;
use betterlite_oracle_helper_api::error::HelperError;

const NORMALIZERS: NameNormalizers = NameNormalizers {
    bedrock_base_model: get_bedrock_base_model,
    bedrock_routing_prefix: strip_bedrock_routing_prefix,
    mantle_region_split: split_region_prefix,
};

// The cases name no huggingface model, so the transport refuses instead of reaching the network.
struct Offline;

#[derive(Debug, thiserror::Error)]
#[error("the metadata lookup must not fetch a Hugging Face config")]
struct OfflineError;

impl ModelInfoClient for Offline {
    type Error = OfflineError;

    fn get(
        &self,
        _request: &ModelInfoRequest,
    ) -> impl Future<Output = Result<FetchResponse, OfflineError>> + Send {
        ready(Err(OfflineError))
    }
}

// B2225's get_model_info over the case's own cost map, so normalize_reasoning_effort_value's lookup is the production one.
pub(super) struct Metadata {
    pub(super) map: ModelCostMap,
    typed: ModelCatalog,
    generalizations: CatalogGeneralizations,
    known: BTreeSet<String>,
    custom: BTreeSet<String>,
    environment: BTreeMap<String, String>,
    hf: HfConfigClient<Offline>,
}

impl Metadata {
    pub(super) fn load(model_cost: &IndexMap<String, Value>) -> Result<Self, Box<HelperError>> {
        let bytes = serde_json::to_vec(model_cost)
            .map_err(|source| helper_error("InvalidInput", None, source.to_string()))?;
        Ok(Self {
            map: ModelCostMap::parse(&bytes)
                .map_err(|source| helper_error("InvalidInput", None, source.to_string()))?,
            // every case names its provider, so resolution never reads the typed catalog
            typed: ModelCatalog::parse(b"{}", UnknownFields::Warn)
                .map_err(|source| helper_error("InvalidInput", None, source.to_string()))?,
            generalizations: CatalogGeneralizations::default(),
            known: BTreeSet::new(),
            custom: BTreeSet::new(),
            environment: BTreeMap::new(),
            hf: HfConfigClient {
                client: Offline,
                base_url: HF_CONFIG_BASE_URL.to_owned(),
            },
        })
    }

    // get_model_info raises for an unmapped model; the caller's facade keeps the chain floor for that None.
    pub(super) fn model_info(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
        runtime: &Runtime,
    ) -> Option<ModelInfo> {
        let resolution = ResolutionContext {
            catalog: &self.typed,
            known_model_names: &self.known,
            custom_providers: &self.custom,
            environment: &self.environment,
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        };
        let environment = MetadataEnvironment {
            model_cost: &self.map.entries,
            vertex: &self.map.vertex,
            generalizations: &self.generalizations,
            resolution: &resolution,
            normalizers: NORMALIZERS,
        };
        let request = MetadataRequest {
            model,
            custom_llm_provider,
            api_base: None,
            api_key: None,
        };
        let metadata = runtime
            .block_on(get_model_info_helper(
                &request,
                &environment,
                &mut ModelCostKeys::default(),
                &NoDynamicMetadata,
                &self.hf,
            ))
            .ok()?;
        ModelInfo::parse(&metadata).ok()
    }
}
