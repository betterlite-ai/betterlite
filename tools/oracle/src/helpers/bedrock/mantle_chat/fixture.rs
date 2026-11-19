use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::io;
use std::sync::Arc;

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::headers::AnthropicHeaders;
use betterlite_anthropic::option_primitives::OptionDefaults;
use betterlite_bedrock::claude_invoke::error::ClaudeInvokeError;
use betterlite_bedrock::claude_invoke::request::{ClaudeInvokeRequest, TransformSources};
use betterlite_bedrock::event_frames::beta::BetaHeader;
use betterlite_bedrock::mantle_chat::async_request::async_transform_request as async_mantle_request;
use betterlite_bedrock::mantle_chat::error::MantleMessagesError;
use betterlite_bedrock::mantle_chat::messages::transform_request as transform_messages_request;
use betterlite_bedrock::mantle_chat::request::transform_request as transform_mantle_request;
use betterlite_bedrock::mantle_inputs::region::split_region_prefix;
use betterlite_bedrock::messages::transform::{
    MessagesRequest, TransformSources as MessagesSources,
};
use betterlite_bedrock::model_routes::ParamMap;
use betterlite_config::sections::anthropic_capability_catalog::Settings;
use betterlite_context::anthropic_catalog_lookup::RawCatalog;
use betterlite_context::anthropic_catalog_lookup::candidates::CandidatePatterns;
use betterlite_context::anthropic_catalog_lookup::error::CatalogLookupError;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_core::api::anthropic_beta_config::matrix::BetaMatrix;
use betterlite_models::catalog::{BundledCatalog, CatalogError, ModelCatalog, UnknownFields};
use betterlite_models::fireworks_catalog_metadata::Catalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
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
use betterlite_prompts::anthropic::assembly::PromptDeps;
use betterlite_prompts::arguments::json::Object;
use indexmap::IndexMap;
use tokio::runtime::{Builder, Runtime};

use super::input::{ChatCall, MessagesCall};
use super::mock::{MapLookup, NoFileBlocks, NoHfTransport, RecordingFetcher};

const BUNDLED_BETA_CONFIG: &str = include_str!(
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
    lookup: MapLookup,
    catalog: ModelCatalog,
    pub(super) model_cost: ParamMap,
    pub(super) budgets: Settings,
    pub(super) matrix: BetaMatrix,
    defaults: OptionDefaults,
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
            lookup: MapLookup::from_cost(model_cost)?,
            catalog: ModelCatalog::parse(b"{}", UnknownFields::Warn)?,
            model_cost: model_cost.clone(),
            budgets: Settings::default(),
            matrix: serde_json::from_str(BUNDLED_BETA_CONFIG)?,
            defaults: OptionDefaults::default(),
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

    pub(super) fn messages_transform(
        &mut self,
        call: &MessagesCall,
        headers: &mut IndexMap<String, String>,
    ) -> Result<Object, MantleMessagesError> {
        let catalogs = self.fixture.catalogs();
        let mut environment =
            self.fixture
                .capability_environment(&self.resolution, &mut self.keys, &self.dynamic);
        let beta = beta_headers(&call.request_headers);
        let request = MessagesRequest {
            model: &call.model,
            messages: call.messages.clone(),
            optional_params: call.optional_params.clone(),
            headers: &beta,
            model_cost: &self.fixture.model_cost,
            drop_params: false,
        };
        let mut sources = MessagesSources {
            environment: &mut environment,
            catalogs: &catalogs,
            budgets: &self.fixture.budgets,
            matrix: &self.fixture.matrix,
            lookup: &self.fixture.lookup,
            packaged: self.fixture.packaged.as_ref(),
        };
        self.runtime
            .block_on(transform_messages_request(&request, headers, &mut sources))
    }

    pub(super) fn chat_transform(
        &mut self,
        call: &ChatCall,
        fetcher: &RecordingFetcher,
        inline: bool,
    ) -> Result<Object, ClaudeInvokeError<Infallible>> {
        let request = ClaudeInvokeRequest {
            model: &call.model,
            messages: call.messages.clone(),
            optional_params: call.optional_params.clone(),
            litellm_params: call.litellm_params.clone(),
            headers: headers(&call.headers),
            defaults: &self.fixture.defaults,
            catalog: &self.fixture.catalog,
            default_chat_max_tokens: 4096,
            modify_params: false,
            global_drop_params: false,
        };
        let catalogs = self.fixture.catalogs();
        let blocks = NoFileBlocks;
        let new_id: fn() -> String = || "generated".to_owned();
        let deps = PromptDeps {
            fetcher,
            blocks: &blocks,
            new_id: &new_id,
        };
        let mut environment =
            self.fixture
                .capability_environment(&self.resolution, &mut self.keys, &self.dynamic);
        let mut sources = TransformSources {
            environment: &mut environment,
            catalogs: &catalogs,
            prompts: &deps,
            matrix: &self.fixture.matrix,
            lookup: &self.fixture.lookup,
            packaged: self.fixture.packaged.as_ref(),
        };
        self.runtime.block_on(async {
            if inline {
                async_mantle_request(&request, &mut sources).await
            } else {
                transform_mantle_request(&request, &mut sources).await
            }
        })
    }
}

fn beta_headers(entries: &IndexMap<String, String>) -> BTreeMap<String, BetaHeader> {
    entries
        .iter()
        .map(|(name, value)| (name.clone(), BetaHeader::Text(value.clone())))
        .collect()
}

fn headers(entries: &BTreeMap<String, String>) -> AnthropicHeaders {
    let mut headers = AnthropicHeaders::default();
    for (name, value) in entries {
        headers.values.insert(name.clone(), value.clone());
    }
    headers
}
