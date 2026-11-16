use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::sync::Arc;

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_bedrock::bedrock_messages_beta::beta;
use betterlite_bedrock::bedrock_messages_beta::beta::{BetaHeadersError, BetaHeadersRequest};
use betterlite_bedrock::bedrock_model_capabilities::tool_search::supports_tool_search;
use betterlite_bedrock::mantle_inputs::region::split_region_prefix;
use betterlite_bedrock::model_routes::ParamMap;
use betterlite_context::anthropic_catalog_lookup::RawCatalog;
use betterlite_context::anthropic_catalog_lookup::candidates::CandidatePatterns;
use betterlite_context::anthropic_catalog_lookup::error::CatalogLookupError;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_core::api::anthropic_beta_config::matrix::BetaMatrix;
use betterlite_models::catalog::{BundledCatalog, CatalogError, ModelCatalog, UnknownFields};
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
use std::io;
use tokio::runtime::{Builder, Runtime};

// The same bundled config the production loader reads, byte-identical to LiteLLM's local backup.
pub(super) const BUNDLED_BETA_CONFIG: &str = include_str!(
    "../../../../../../crates/betterlite-core/src/api/anthropic_beta_config/anthropic_beta_headers_config.json"
);

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
    #[error("runtime: {0}")]
    Runtime(#[from] io::Error),
}

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
    packaged: Arc<ModelCatalog>,
    provider_catalog: Catalog,
    hf: HfConfigClient<NoHfTransport>,
    known_model_names: BTreeSet<String>,
    custom_providers: BTreeSet<String>,
    environment: BTreeMap<String, String>,
    patterns: CandidatePatterns,
    runtime: RawCatalog,
    unlisted: RawCatalog,
}

impl Fixture {
    pub(super) fn load(model_cost: &ParamMap) -> Result<Self, FixtureError> {
        let bytes = serde_json::to_vec(model_cost)?;
        let mut bundled = BundledCatalog::default();
        Ok(Self {
            entries: ModelCostMap::parse(&bytes)?,
            typed: ModelCatalog::parse(b"{}", UnknownFields::Warn)?,
            packaged: bundled.get(),
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
        })
    }

    const fn catalogs(&self) -> CatalogSources<'_> {
        CatalogSources {
            patterns: &self.patterns,
            runtime: &self.runtime,
            packaged: &self.unlisted,
        }
    }

    fn resolution(&self) -> ResolutionContext<'_> {
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

    fn capability_environment<'fixture>(
        &'fixture self,
        resolution: &'fixture ResolutionContext<'fixture>,
        keys: &'fixture mut ModelCostKeys,
        dynamic: &'fixture NoDynamicMetadata,
    ) -> CapabilityEnvironment<'fixture, NoDynamicMetadata, NoHfTransport> {
        CapabilityEnvironment {
            metadata: MetadataEnvironment {
                model_cost: &self.entries.entries,
                vertex: &self.entries.vertex,
                generalizations: &self.packaged.generalizations,
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

pub(super) struct Session<'fixture> {
    fixture: &'fixture Fixture,
    resolution: ResolutionContext<'fixture>,
    keys: ModelCostKeys,
    dynamic: NoDynamicMetadata,
    runtime: Runtime,
}

impl<'fixture> Session<'fixture> {
    pub(super) fn new(fixture: &'fixture Fixture) -> Result<Self, FixtureError> {
        Ok(Self {
            fixture,
            resolution: fixture.resolution(),
            keys: ModelCostKeys::default(),
            dynamic: NoDynamicMetadata,
            runtime: Builder::new_current_thread().build()?,
        })
    }

    pub(super) fn betas(
        &mut self,
        request: BetaHeadersRequest<'_>,
        matrix: &BetaMatrix,
    ) -> Result<Vec<String>, BetaHeadersError> {
        let catalogs = self.fixture.catalogs();
        let mut environment =
            self.fixture
                .capability_environment(&self.resolution, &mut self.keys, &self.dynamic);
        self.runtime.block_on(beta::beta_headers(
            request,
            &mut environment,
            &catalogs,
            matrix,
        ))
    }

    pub(super) fn supports_tool_search(&mut self, model: &str) -> bool {
        let catalogs = self.fixture.catalogs();
        let mut environment =
            self.fixture
                .capability_environment(&self.resolution, &mut self.keys, &self.dynamic);
        self.runtime
            .block_on(supports_tool_search(model, &mut environment, &catalogs))
    }
}
