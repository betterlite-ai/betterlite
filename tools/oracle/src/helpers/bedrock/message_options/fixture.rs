use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::{Arc, PoisonError};

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_bedrock::bedrock_model_capabilities::ModelInfoLookup;
use betterlite_bedrock::mantle_inputs::region::split_region_prefix;
use betterlite_bedrock::model_routes::ParamMap;
use betterlite_context::anthropic_catalog_lookup::RawCatalog;
use betterlite_context::anthropic_catalog_lookup::candidates::CandidatePatterns;
use betterlite_context::anthropic_catalog_lookup::error::CatalogLookupError;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_models::catalog::{BundledCatalog, CatalogError, ModelCatalog, UnknownFields};
use betterlite_models::fetch_policy::response::FetchResponse;
use betterlite_models::fireworks_catalog_metadata::Catalog;
use betterlite_models::full_model_metadata::cache::ModelInfoCache;
use betterlite_models::full_model_metadata::error::FullMetadataError;
use betterlite_models::full_model_metadata::record::ModelInfo;
use betterlite_models::full_model_metadata::{
    HelperQuery, HelperSources, get_model_info_helper_cached,
};
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

const METADATA_CACHE_SIZE: i64 = 128;

const NORMALIZERS: NameNormalizers = NameNormalizers {
    bedrock_base_model: get_bedrock_base_model,
    bedrock_routing_prefix: strip_bedrock_routing_prefix,
    mantle_region_split: split_region_prefix,
};

#[derive(Debug, thiserror::Error)]
pub(super) enum FixtureError {
    #[error("fixture JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("model cost map: {0}")]
    ModelCost(#[from] ModelCostMapError),
    #[error("model catalog: {0}")]
    Catalog(#[from] CatalogError),
    #[error("candidate patterns: {0}")]
    Patterns(#[from] CatalogLookupError),
}

// The cases name no huggingface model, so the transport refuses rather than reaching the network.
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
    entries: ModelCostMap,
    typed: ModelCatalog,
    provider_catalog: Catalog,
    hf: HfConfigClient<NoHfTransport>,
    known_model_names: BTreeSet<String>,
    custom_providers: BTreeSet<String>,
    environment: BTreeMap<String, String>,
    patterns: CandidatePatterns,
    runtime: RawCatalog,
    unlisted: RawCatalog,
    pub(super) packaged: Arc<ModelCatalog>,
}

impl Fixture {
    pub(super) fn load(model_cost: &ParamMap) -> Result<Self, FixtureError> {
        let bytes = serde_json::to_vec(model_cost)?;
        Ok(Self {
            entries: ModelCostMap::parse(&bytes)?,
            // every case names its provider, so resolution never reads the typed catalog
            typed: ModelCatalog::parse(b"{}", UnknownFields::Warn)?,
            provider_catalog: Catalog::new(),
            hf: HfConfigClient {
                client: NoHfTransport,
                base_url: HF_CONFIG_BASE_URL.to_owned(),
            },
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            environment: BTreeMap::new(),
            patterns: CandidatePatterns::new()?,
            runtime: serde_json::from_slice(&bytes)?,
            unlisted: RawCatalog::new(),
            packaged: BundledCatalog::default().get(),
        })
    }

    pub(super) const fn catalogs(&self) -> CatalogSources<'_> {
        CatalogSources {
            patterns: &self.patterns,
            runtime: &self.runtime,
            packaged: &self.unlisted,
        }
    }

    pub(super) fn resolution(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.typed,
            known_model_names: &self.known_model_names,
            custom_providers: &self.custom_providers,
            environment: &self.environment,
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }

    pub(super) fn metadata_environment<'fixture>(
        &'fixture self,
        resolution: &'fixture ResolutionContext<'fixture>,
    ) -> MetadataEnvironment<'fixture> {
        MetadataEnvironment {
            model_cost: &self.entries.entries,
            vertex: &self.entries.vertex,
            generalizations: &self.packaged.generalizations,
            resolution,
            normalizers: NORMALIZERS,
        }
    }

    pub(super) fn capability_environment<'fixture>(
        &'fixture self,
        resolution: &'fixture ResolutionContext<'fixture>,
        keys: &'fixture mut ModelCostKeys,
        dynamic: &'fixture NoDynamicMetadata,
    ) -> CapabilityEnvironment<'fixture, NoDynamicMetadata, NoHfTransport> {
        CapabilityEnvironment {
            metadata: self.metadata_environment(resolution),
            keys,
            dynamic,
            hf: &self.hf,
            provider_catalog: &self.provider_catalog,
            provider_generation: 0,
        }
    }
}

// The effort ceiling reads the model-info helper, the same resolution the capability gate uses.
pub(super) struct CostLookup<'fixture> {
    pub(super) fixture: &'fixture Fixture,
    pub(super) resolution: &'fixture ResolutionContext<'fixture>,
}

impl ModelInfoLookup for CostLookup<'_> {
    type Error = FullMetadataError<Infallible>;

    async fn bedrock_model_info(&self, model: &str) -> Result<ModelInfo, Self::Error> {
        let environment = self.fixture.metadata_environment(self.resolution);
        let dynamic = NoDynamicMetadata;
        let sources = HelperSources {
            environment: &environment,
            dynamic: &dynamic,
            hf: &self.fixture.hf,
        };
        let query = HelperQuery {
            model,
            custom_llm_provider: Some("bedrock"),
            api_base: None,
        };
        let mut keys = ModelCostKeys::default();
        let cache = ModelInfoCache::new(METADATA_CACHE_SIZE);
        let handle = get_model_info_helper_cached(&query, &sources, &mut keys, &cache).await?;
        let info = handle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        Ok(info)
    }
}
