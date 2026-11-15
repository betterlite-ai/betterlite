use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::headers::AnthropicHeaders;
use betterlite_anthropic::option_primitives::OptionDefaults;
use betterlite_anthropic::params::DEFAULT_MAX_TOKENS;
use betterlite_anthropic::request::error::RequestError;
use betterlite_anthropic::request::{AnthropicRequest, PreparedRequest};
use betterlite_bedrock::mantle_inputs::region::split_region_prefix;
use betterlite_context::anthropic_catalog_lookup::RawCatalog;
use betterlite_context::anthropic_catalog_lookup::candidates::CandidatePatterns;
use betterlite_context::anthropic_catalog_lookup::error::CatalogLookupError;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_models::catalog::{CatalogError, ModelCatalog, UnknownFields};
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
use betterlite_models::model_metadata_resolution::names::{
    ModelCostMap, ModelCostMapError, NameNormalizers,
};
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_prompts::anthropic::assembly::PromptDeps;
use betterlite_prompts::anthropic_results::tool_result::FileBlocks;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::value::RawValue;

use super::super::Input;

const ID: &str = "generated-id";

const NORMALIZERS: NameNormalizers = NameNormalizers {
    bedrock_base_model: get_bedrock_base_model,
    bedrock_routing_prefix: strip_bedrock_routing_prefix,
    mantle_region_split: split_region_prefix,
};

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FixtureInput {
    #[serde(default)]
    pub(super) model_cost: IndexMap<String, Box<RawValue>>,
    #[serde(default)]
    pub(super) packaged: serde_json::Value,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum FixtureError {
    #[error("fixture JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("model cost map: {0}")]
    ModelCost(#[from] ModelCostMapError),
    #[error("model catalog: {0}")]
    Catalog(#[from] CatalogError),
    #[error("candidate patterns: {0}")]
    Patterns(#[from] CatalogLookupError),
}

// The cases name no huggingface model, so the transport refuses instead of reaching the network.
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

// The current no-download seam: a selected image URL is refused with the pinned 400 fetch error
pub(crate) struct RefusingFetcher;

fn refused(url: &str) -> ImageFetchError {
    ImageFetchError::Disabled {
        url: url.to_owned(),
    }
}

impl RemoteMediaFetcher for RefusingFetcher {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        ready(Err(refused(url)))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        ready(Err(refused(urls.first().map_or("", String::as_str))))
    }
}

pub(crate) struct NoFileBlocks;

#[derive(Debug, thiserror::Error)]
#[error("the case supplies no file blocks")]
pub(crate) struct NoFileError;

impl FileBlocks for NoFileBlocks {
    type Failure = NoFileError;

    fn file_block(&self, _element: &Object) -> Result<Object, NoFileError> {
        Err(NoFileError)
    }
}

pub(crate) struct Records {
    map: ModelCostMap,
    typed: ModelCatalog,
    generalizations: CatalogGeneralizations,
    provider_catalog: Catalog,
    hf: HfConfigClient<NoHfTransport>,
    known: BTreeSet<String>,
    custom: BTreeSet<String>,
    environment: BTreeMap<String, String>,
    patterns: CandidatePatterns,
    runtime: RawCatalog,
    packaged: RawCatalog,
}

impl Records {
    pub(crate) fn load(input: &FixtureInput) -> Result<Self, FixtureError> {
        let bytes = serde_json::to_vec(&input.model_cost)?;
        Ok(Self {
            map: ModelCostMap::parse(&bytes)?,
            // every case names its provider, so resolution never reads the typed catalog
            typed: ModelCatalog::parse(b"{}", UnknownFields::Warn)?,
            generalizations: CatalogGeneralizations::default(),
            provider_catalog: Catalog::new(),
            hf: HfConfigClient {
                client: NoHfTransport,
                base_url: HF_CONFIG_BASE_URL.to_owned(),
            },
            known: BTreeSet::new(),
            custom: BTreeSet::new(),
            environment: BTreeMap::new(),
            patterns: CandidatePatterns::new()?,
            runtime: serde_json::from_slice(&bytes)?,
            packaged: serde_json::from_value(input.packaged.clone())?,
        })
    }

    const fn catalogs(&self) -> CatalogSources<'_> {
        CatalogSources {
            patterns: &self.patterns,
            runtime: &self.runtime,
            packaged: &self.packaged,
        }
    }

    fn resolution(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.typed,
            known_model_names: &self.known,
            custom_providers: &self.custom,
            environment: &self.environment,
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }

    const fn environment<'fixture>(
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

    pub(crate) async fn prepare(
        &self,
        input: &Input,
    ) -> Result<PreparedRequest, RequestError<NoFileError>> {
        let catalogs = self.catalogs();
        let resolution = self.resolution();
        let mut keys = ModelCostKeys::default();
        let dynamic = NoDynamicMetadata;
        let mut environment = self.environment(&resolution, &mut keys, &dynamic);
        let defaults = OptionDefaults::default();
        let request = AnthropicRequest {
            model: &input.model,
            custom_llm_provider: input.custom_llm_provider.as_deref(),
            messages: input.messages.clone(),
            optional_params: input.optional_params.clone(),
            litellm_params: input.litellm_params.clone(),
            headers: AnthropicHeaders {
                values: input.headers.clone(),
            },
            defaults: &defaults,
            catalog: &self.typed,
            default_chat_max_tokens: DEFAULT_MAX_TOKENS,
            billing_header: input.strips_billing_metadata,
            modify_params: input.modify_params,
            global_drop_params: input.global_drop_params,
        };
        let fetcher = RefusingFetcher;
        let blocks = NoFileBlocks;
        let new_id = || ID.to_owned();
        let prompts = PromptDeps {
            fetcher: &fetcher,
            blocks: &blocks,
            new_id: &new_id,
        };
        request.prepare(&mut environment, &catalogs, &prompts).await
    }
}
