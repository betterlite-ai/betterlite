use std::collections::{BTreeMap, BTreeSet};

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::headers::AnthropicHeaders;
use betterlite_anthropic::option_primitives::OptionDefaults;
use betterlite_anthropic::request::error::RequestError;
use betterlite_bedrock::claude_platform::chat::{
    ChatHeadersRequest, ChatPrepared, ChatRequest, ChatSources, HeaderSources,
    validate_environment as validate_chat,
};
use betterlite_bedrock::claude_platform::error::{ChatValidationError, NativeTransformError};
use betterlite_bedrock::claude_platform::native::{
    NativePrepared, NativeRequest, NativeSources, transform_request as native_transform,
};
use betterlite_bedrock::mantle_inputs::region::split_region_prefix;
use betterlite_config::sections::anthropic_capability_catalog::Settings as Budgets;
use betterlite_context::anthropic_catalog_lookup::RawCatalog;
use betterlite_context::anthropic_catalog_lookup::candidates::CandidatePatterns;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::catalog_generalizations::CatalogGeneralizations;
use betterlite_models::decisions::Json;
use betterlite_models::fireworks_catalog_metadata::Catalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::MetadataEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::NoDynamicMetadata;
use betterlite_models::model_metadata_resolution::hf_config::{HF_CONFIG_BASE_URL, HfConfigClient};
use betterlite_models::model_metadata_resolution::names::{ModelCostMap, NameNormalizers};
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_prompts::anthropic::assembly::PromptDeps;
use betterlite_prompts::arguments::json::Json as PromptJson;
use indexmap::IndexMap;
use serde_json::json;
use tokio::runtime::{Builder, Runtime};

use super::stubs::{NoFileBlocks, NoFileError, NoHfTransport, RefusingFetcher, invalid};

pub(super) const GENERATED_ID: &str = "generated-id";

const NORMALIZERS: NameNormalizers = NameNormalizers {
    bedrock_base_model: get_bedrock_base_model,
    bedrock_routing_prefix: strip_bedrock_routing_prefix,
    mantle_region_split: split_region_prefix,
};

pub(super) struct Fixture {
    map: ModelCostMap,
    typed: ModelCatalog,
    generalizations: CatalogGeneralizations,
    provider_catalog: Catalog,
    hf: HfConfigClient<NoHfTransport>,
    known: BTreeSet<String>,
    custom: BTreeSet<String>,
    environment: BTreeMap<String, String>,
    patterns: CandidatePatterns,
    runtime_catalog: RawCatalog,
    packaged_catalog: RawCatalog,
    budgets: Budgets,
    runtime: Runtime,
}

impl Fixture {
    pub(super) fn empty() -> Result<Self, Box<HelperError>> {
        let raw = json!({});
        Ok(Self {
            map: ModelCostMap::parse(b"{}").map_err(invalid)?,
            typed: ModelCatalog::parse(b"{}", UnknownFields::Warn).map_err(invalid)?,
            generalizations: CatalogGeneralizations::default(),
            provider_catalog: Catalog::new(),
            hf: HfConfigClient {
                client: NoHfTransport,
                base_url: HF_CONFIG_BASE_URL.to_owned(),
            },
            known: BTreeSet::new(),
            custom: BTreeSet::new(),
            environment: BTreeMap::new(),
            patterns: CandidatePatterns::new().map_err(invalid)?,
            runtime_catalog: serde_json::from_value(raw.clone()).map_err(invalid)?,
            packaged_catalog: serde_json::from_value(raw).map_err(invalid)?,
            budgets: Budgets::default(),
            runtime: Builder::new_current_thread().build().map_err(invalid)?,
        })
    }

    const fn catalogs(&self) -> CatalogSources<'_> {
        CatalogSources {
            patterns: &self.patterns,
            runtime: &self.runtime_catalog,
            packaged: &self.packaged_catalog,
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

    fn parts(
        &self,
    ) -> (
        CatalogSources<'_>,
        ModelCostKeys,
        NoDynamicMetadata,
        ResolutionContext<'_>,
    ) {
        (
            self.catalogs(),
            ModelCostKeys::default(),
            NoDynamicMetadata,
            self.resolution(),
        )
    }

    pub(super) fn chat_headers(
        &self,
        request: &ChatHeadersRequest<'_>,
    ) -> Result<AnthropicHeaders, ChatValidationError> {
        let (catalogs, mut keys, dynamic, resolution) = self.parts();
        let mut environment = self.environment(&resolution, &mut keys, &dynamic);
        let mut sources = HeaderSources {
            environment: &mut environment,
            catalogs: &catalogs,
        };
        self.runtime.block_on(validate_chat(request, &mut sources))
    }

    pub(super) fn chat_body(
        &self,
        model: &str,
        messages: Vec<Json>,
        optional_params: IndexMap<String, Json>,
        litellm_params: IndexMap<String, Json>,
        headers: IndexMap<String, String>,
    ) -> Result<ChatPrepared, RequestError<NoFileError>> {
        let (catalogs, mut keys, dynamic, resolution) = self.parts();
        let mut environment = self.environment(&resolution, &mut keys, &dynamic);
        let fetcher = RefusingFetcher;
        let blocks = NoFileBlocks;
        let new_id: fn() -> String = || GENERATED_ID.to_owned();
        let defaults = OptionDefaults::default();
        let prompts = PromptDeps {
            fetcher: &fetcher,
            blocks: &blocks,
            new_id: &new_id,
        };
        let request = ChatRequest {
            model,
            messages,
            optional_params,
            litellm_params,
            headers: AnthropicHeaders { values: headers },
            defaults: &defaults,
            catalog: &self.typed,
            default_chat_max_tokens: 4096,
            modify_params: false,
            global_drop_params: false,
        };
        self.runtime.block_on(
            betterlite_bedrock::claude_platform::chat::transform_request(
                &request,
                &mut ChatSources {
                    environment: &mut environment,
                    catalogs: &catalogs,
                    prompts: &prompts,
                },
            ),
        )
    }

    pub(super) fn native_body(
        &self,
        model: &str,
        messages: Vec<PromptJson>,
        optional_params: IndexMap<String, PromptJson>,
        litellm_params: IndexMap<String, Json>,
    ) -> Result<NativePrepared, NativeTransformError> {
        let (catalogs, mut keys, dynamic, resolution) = self.parts();
        let mut environment = self.environment(&resolution, &mut keys, &dynamic);
        self.runtime.block_on(native_transform(
            &NativeRequest {
                model,
                messages,
                optional_params,
                litellm_params,
            },
            &mut NativeSources {
                environment: &mut environment,
                catalogs: &catalogs,
                budgets: &self.budgets,
            },
        ))
    }
}
