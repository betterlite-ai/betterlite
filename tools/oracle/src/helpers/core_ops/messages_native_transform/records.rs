use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_config::sections::anthropic_capability_catalog::Settings;
use betterlite_context::anthropic_catalog_lookup::RawCatalog;
use betterlite_context::anthropic_catalog_lookup::candidates::CandidatePatterns;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_core::api::messages_native_transform::capabilities::AnthropicCapabilities;
use betterlite_core::api::messages_native_transform::error::TransformError;
use betterlite_core::api::messages_native_transform::request::{
    TransformRequest, transform_native_request,
};
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::catalog_generalizations::CatalogGeneralizations;
use betterlite_models::fetch_policy::response::FetchResponse;
use betterlite_models::fireworks_catalog_metadata::Catalog;
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
use betterlite_prompts::arguments::json::Object;
use indexmap::IndexMap;
use serde_json::Value;
use tokio::runtime::Runtime;

use super::invalid;
use crate::helpers::HelperError;

fn region_model(model: &str) -> (Option<&str>, &str) {
    match model.split_once('/') {
        Some((region, rest)) if region == "us-east-1" => (Some(region), rest),
        Some(_) | None => (None, model),
    }
}

const NORMALIZERS: NameNormalizers = NameNormalizers {
    bedrock_base_model: get_bedrock_base_model,
    bedrock_routing_prefix: strip_bedrock_routing_prefix,
    mantle_region_split: region_model,
};

// The cases name no huggingface model, so the transport refuses instead of reaching the network.
pub(super) struct Offline;

#[derive(Debug, thiserror::Error)]
#[error("the metadata lookup must not fetch a Hugging Face config")]
pub(super) struct OfflineError;

impl ModelInfoClient for Offline {
    type Error = OfflineError;

    fn get(
        &self,
        _request: &ModelInfoRequest,
    ) -> impl Future<Output = Result<FetchResponse, OfflineError>> + Send {
        ready(Err(OfflineError))
    }
}

pub(super) struct Records {
    map: ModelCostMap,
    typed: ModelCatalog,
    generalizations: CatalogGeneralizations,
    provider_catalog: Catalog,
    hf: HfConfigClient<Offline>,
    known: BTreeSet<String>,
    custom: BTreeSet<String>,
    environment: BTreeMap<String, String>,
    patterns: CandidatePatterns,
    runtime: RawCatalog,
    packaged: RawCatalog,
    budgets: Settings,
}

impl Records {
    pub(super) fn load(
        model_cost: &IndexMap<String, Value>,
        packaged: &IndexMap<String, Value>,
    ) -> Result<Self, Box<HelperError>> {
        let cost_bytes =
            serde_json::to_vec(model_cost).map_err(|source| invalid(source.to_string()))?;
        let packaged_bytes =
            serde_json::to_vec(packaged).map_err(|source| invalid(source.to_string()))?;
        Ok(Self {
            map: ModelCostMap::parse(&cost_bytes).map_err(|source| invalid(source.to_string()))?,
            // every case names its provider, so resolution never reads the typed catalog
            typed: ModelCatalog::parse(b"{}", UnknownFields::Warn)
                .map_err(|source| invalid(source.to_string()))?,
            generalizations: CatalogGeneralizations::default(),
            provider_catalog: Catalog::new(),
            hf: HfConfigClient {
                client: Offline,
                base_url: HF_CONFIG_BASE_URL.to_owned(),
            },
            known: BTreeSet::new(),
            custom: BTreeSet::new(),
            environment: BTreeMap::new(),
            patterns: CandidatePatterns::new().map_err(|source| invalid(source.to_string()))?,
            runtime: serde_json::from_slice(&cost_bytes)
                .map_err(|source| invalid(source.to_string()))?,
            packaged: serde_json::from_slice(&packaged_bytes)
                .map_err(|source| invalid(source.to_string()))?,
            budgets: Settings::default(),
        })
    }

    const fn catalogs(&self) -> CatalogSources<'_> {
        CatalogSources {
            patterns: &self.patterns,
            runtime: &self.runtime,
            packaged: &self.packaged,
        }
    }

    pub(super) fn call(
        &self,
        request: TransformRequest<'_>,
        runtime: &Runtime,
    ) -> Result<Object, TransformError> {
        let catalogs = self.catalogs();
        let resolution = ResolutionContext {
            catalog: &self.typed,
            known_model_names: &self.known,
            custom_providers: &self.custom,
            environment: &self.environment,
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        };
        let mut keys = ModelCostKeys::default();
        let dynamic = NoDynamicMetadata;
        let mut environment = CapabilityEnvironment {
            metadata: MetadataEnvironment {
                model_cost: &self.map.entries,
                vertex: &self.map.vertex,
                generalizations: &self.generalizations,
                resolution: &resolution,
                normalizers: NORMALIZERS,
            },
            keys: &mut keys,
            dynamic: &dynamic,
            hf: &self.hf,
            provider_catalog: &self.provider_catalog,
            provider_generation: 0,
        };
        let mut capabilities = AnthropicCapabilities {
            environment: &mut environment,
            catalogs: &catalogs,
            budgets: &self.budgets,
            llm_provider: request.custom_llm_provider,
        };
        runtime.block_on(transform_native_request(&mut capabilities, request))
    }
}
