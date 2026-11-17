use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_context::anthropic_catalog_lookup::RawCatalog;
use betterlite_context::anthropic_catalog_lookup::candidates::CandidatePatterns;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::catalog_generalizations::CatalogGeneralizations;
use betterlite_models::decisions::Json;
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
use betterlite_prompts::bedrock_tools::capabilities::BedrockToolCapabilities;
use betterlite_prompts::bedrock_tools::name::ToolNameMap;
use indexmap::IndexMap;
use tokio::runtime::{Builder, Runtime};

use crate::bedrock_converse_tools_beta::error::ToolsAndBetaError;
use crate::bedrock_converse_tools_beta::{ToolsAndBetaRequest, process_tools_and_beta};
use crate::event_frames::beta::BetaHeader;
use crate::mantle_inputs::region::split_region_prefix;

const NORMALIZERS: NameNormalizers = NameNormalizers {
    bedrock_base_model: get_bedrock_base_model,
    bedrock_routing_prefix: strip_bedrock_routing_prefix,
    mantle_region_split: split_region_prefix,
};

pub(super) fn runtime() -> Runtime {
    Builder::new_current_thread()
        .build()
        .expect("the test runtime builds")
}

pub(super) struct Fixture {
    entries: ModelCostMap,
    typed: ModelCatalog,
    generalizations: CatalogGeneralizations,
    provider_catalog: Catalog,
    hf: HfConfigClient<NoHfTransport>,
    known_model_names: BTreeSet<String>,
    custom_providers: BTreeSet<String>,
    variables: BTreeMap<String, String>,
    patterns: CandidatePatterns,
    runtime_catalog: RawCatalog,
    unlisted: RawCatalog,
}

impl Fixture {
    pub(super) fn new(model_cost: &str) -> Self {
        Self {
            entries: ModelCostMap::parse(model_cost.as_bytes()).expect("the model cost map parses"),
            typed: ModelCatalog::parse(b"{}", UnknownFields::Warn)
                .expect("an empty catalog parses"),
            generalizations: CatalogGeneralizations::default(),
            provider_catalog: Catalog::new(),
            hf: HfConfigClient {
                client: NoHfTransport,
                base_url: HF_CONFIG_BASE_URL.to_owned(),
            },
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            variables: BTreeMap::new(),
            patterns: CandidatePatterns::new().expect("the candidate patterns compile"),
            runtime_catalog: serde_json::from_slice(model_cost.as_bytes())
                .expect("the raw catalog parses"),
            unlisted: RawCatalog::new(),
        }
    }

    pub(super) fn session(&self) -> Session<'_> {
        Session {
            fixture: self,
            resolution: self.resolution(),
            keys: ModelCostKeys::default(),
            dynamic: NoDynamicMetadata,
        }
    }

    fn resolution(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.typed,
            known_model_names: &self.known_model_names,
            custom_providers: &self.custom_providers,
            environment: &self.variables,
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }

    const fn catalogs(&self) -> CatalogSources<'_> {
        CatalogSources {
            patterns: &self.patterns,
            runtime: &self.runtime_catalog,
            packaged: &self.unlisted,
        }
    }

    pub(super) fn environment<'session>(
        &'session self,
        resolution: &'session ResolutionContext<'session>,
        keys: &'session mut ModelCostKeys,
        dynamic: &'session NoDynamicMetadata,
    ) -> CapabilityEnvironment<'session, NoDynamicMetadata, NoHfTransport> {
        CapabilityEnvironment {
            metadata: MetadataEnvironment {
                model_cost: &self.entries.entries,
                vertex: &self.entries.vertex,
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

pub(super) struct Session<'fixture> {
    fixture: &'fixture Fixture,
    resolution: ResolutionContext<'fixture>,
    keys: ModelCostKeys,
    dynamic: NoDynamicMetadata,
}

pub(super) struct Request<'source> {
    pub(super) model: &'source str,
    pub(super) tools: &'source str,
    pub(super) headers: Option<&'source str>,
    pub(super) params: &'source str,
}

impl<'source> Request<'source> {
    pub(super) const fn new(
        model: &'source str,
        tools: &'source str,
        headers: Option<&'source str>,
        params: &'source str,
    ) -> Self {
        Self {
            model,
            tools,
            headers,
            params,
        }
    }
}

pub(super) struct Processed {
    pub(super) tools: Vec<Json>,
    pub(super) betas: Vec<Json>,
    pub(super) params: IndexMap<String, Json>,
    pub(super) names: ToolNameMap,
}

pub(super) fn list(source: &str) -> Vec<Json> {
    serde_json::from_str(source).expect("the expected list parses")
}

pub(super) fn params(source: &str) -> IndexMap<String, Json> {
    serde_json::from_str(source).expect("the expected params parse")
}

impl Session<'_> {
    pub(super) fn process(
        &mut self,
        request: &Request<'_>,
    ) -> Result<Processed, ToolsAndBetaError> {
        let tools: Vec<Json> = serde_json::from_str(request.tools)?;
        let headers: Option<BTreeMap<String, BetaHeader>> =
            request.headers.map(serde_json::from_str).transpose()?;
        let mut params: IndexMap<String, Json> = serde_json::from_str(request.params)?;
        let catalogs = self.fixture.catalogs();
        let mut environment =
            self.fixture
                .environment(&self.resolution, &mut self.keys, &self.dynamic);
        let outcome = runtime().block_on(process_tools_and_beta(
            ToolsAndBetaRequest {
                tools: &tools,
                model: request.model,
                headers: headers.as_ref(),
                params: &mut params,
                capabilities: BedrockToolCapabilities::default(),
            },
            &mut environment,
            &catalogs,
        ))?;
        Ok(Processed {
            tools: outcome.tools,
            betas: outcome.betas,
            params,
            names: outcome.names,
        })
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the metadata lookup must not fetch a Hugging Face config")]
pub(super) struct NoHfError;

pub(super) struct NoHfTransport;

impl ModelInfoClient for NoHfTransport {
    type Error = NoHfError;

    fn get(
        &self,
        _request: &ModelInfoRequest,
    ) -> impl Future<Output = Result<FetchResponse, NoHfError>> + Send {
        ready(Err(NoHfError))
    }
}
