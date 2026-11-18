use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::Arc;

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_context::anthropic_catalog_lookup::RawCatalog;
use betterlite_context::anthropic_catalog_lookup::candidates::CandidatePatterns;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::fetch_policy::response::FetchResponse;
use betterlite_models::fireworks_catalog_metadata::Catalog;
use betterlite_models::full_model_metadata::record::ModelInfo;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::lemonade_model_metadata::{ModelInfoClient, ModelInfoRequest};
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::MetadataEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::NoDynamicMetadata;
use betterlite_models::model_metadata_resolution::hf_config::HfConfigClient;
use betterlite_models::model_metadata_resolution::names::{ModelCostMap, NameNormalizers};
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use indexmap::IndexMap;

use super::bedrock::bedrock_model_capabilities::ModelInfoLookup;
use super::bedrock::mantle_inputs::region::split_region_prefix;

pub(super) struct Metadata {
    pub(crate) packaged: Arc<ModelCatalog>,
    pub(crate) map: ModelCostMap,
    pub(crate) names: BTreeSet<String>,
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) catalog: Catalog,
    pub(crate) hf: HfConfigClient<Offline>,
    pub(crate) raw: RawCatalog,
    pub(crate) patterns: CandidatePatterns,
}

impl Metadata {
    pub(crate) fn resolution(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.packaged,
            known_model_names: &self.names,
            custom_providers: &self.names,
            environment: &self.environment,
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }

    pub(crate) const fn catalogs(&self) -> CatalogSources<'_> {
        CatalogSources {
            patterns: &self.patterns,
            runtime: &self.raw,
            packaged: &self.raw,
        }
    }

    pub(crate) fn capabilities<'env>(
        &'env self,
        resolution: &'env ResolutionContext<'env>,
        keys: &'env mut ModelCostKeys,
    ) -> CapabilityEnvironment<'env, NoDynamicMetadata, Offline> {
        CapabilityEnvironment {
            metadata: MetadataEnvironment {
                model_cost: &self.map.entries,
                vertex: &self.map.vertex,
                generalizations: &self.packaged.generalizations,
                resolution,
                normalizers: NameNormalizers {
                    bedrock_base_model: get_bedrock_base_model,
                    bedrock_routing_prefix: strip_bedrock_routing_prefix,
                    mantle_region_split: split_region_prefix,
                },
            },
            keys,
            dynamic: &NoDynamicMetadata,
            hf: &self.hf,
            provider_catalog: &self.catalog,
            provider_generation: 0,
        }
    }
}

pub(super) struct Offline;

#[derive(Debug, thiserror::Error)]
#[error("fixture forbids metadata network calls")]
pub(super) struct Network;

impl ModelInfoClient for Offline {
    type Error = Network;
    fn get(
        &self,
        _request: &ModelInfoRequest,
    ) -> impl Future<Output = Result<FetchResponse, Network>> + Send {
        ready(Err(Network))
    }
}

impl ModelInfoLookup for Offline {
    type Error = Infallible;
    fn bedrock_model_info(
        &self,
        _model: &str,
    ) -> impl Future<Output = Result<ModelInfo, Infallible>> + Send {
        ready(Ok(ModelInfo {
            fields: IndexMap::new(),
        }))
    }
}

pub(super) fn metadata() -> Result<Metadata, super::run::FixtureError> {
    Ok(Metadata {
        packaged: BundledCatalog::default().get(),
        map: ModelCostMap::parse(b"{}")?,
        names: BTreeSet::new(),
        environment: BTreeMap::new(),
        catalog: Catalog::new(),
        hf: HfConfigClient {
            client: Offline,
            base_url: "https://invalid.test".to_owned(),
        },
        raw: RawCatalog::new(),
        patterns: CandidatePatterns::new()?,
    })
}
