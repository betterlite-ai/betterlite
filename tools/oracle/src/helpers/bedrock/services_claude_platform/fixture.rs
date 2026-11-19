use std::collections::{BTreeMap, BTreeSet};

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::headers::AnthropicHeaders;
use betterlite_anthropic::option_primitives::OptionDefaults;
use betterlite_anthropic::request::error::RequestError;
use betterlite_bedrock::claude_platform::chat::{
    ChatHeadersRequest, ChatPrepared, ChatRequest, ChatSources, HeaderSources,
    transform_request as chat_transform, validate_environment as validate_chat,
};
use betterlite_bedrock::claude_platform::convert;
use betterlite_bedrock::claude_platform::error::ChatValidationError;
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
use serde_json::json;
use tokio::runtime::{Builder, Runtime};

use super::input::{BodyInput, HeaderInput};
use super::stubs::{
    NoFileBlocks, NoFileError, NoHfTransport, PlatformInputsCase, RefusingFetcher, invalid,
};

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

    pub(super) fn chat_headers_from(
        &self,
        input: &HeaderInput,
        platform: &PlatformInputsCase,
    ) -> Result<AnthropicHeaders, ChatValidationError> {
        self.chat_headers(&ChatHeadersRequest {
            model: &input.model,
            messages: &input.messages,
            optional_params: &input.optional_params,
            caller_headers: &input.headers,
            api_key: input.api_key.as_ref(),
            workspace: platform.sources(),
        })
    }

    pub(super) fn chat_body(
        &self,
        input: &BodyInput,
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
            model: &input.model,
            messages: input.messages.clone(),
            optional_params: input.optional_params.clone(),
            litellm_params: input.litellm_params.clone(),
            headers: AnthropicHeaders {
                values: input.headers.clone(),
            },
            defaults: &defaults,
            catalog: &self.typed,
            default_chat_max_tokens: 4096,
            modify_params: false,
            global_drop_params: false,
        };
        self.runtime.block_on(chat_transform(
            &request,
            &mut ChatSources {
                environment: &mut environment,
                catalogs: &catalogs,
                prompts: &prompts,
            },
        ))
    }

    pub(super) fn native_body(
        &self,
        input: &BodyInput,
    ) -> Result<NativePrepared, Box<HelperError>> {
        let (catalogs, mut keys, dynamic, resolution) = self.parts();
        let mut environment = self.environment(&resolution, &mut keys, &dynamic);
        let messages = convert::prompt_messages(&input.messages).map_err(invalid)?;
        let optional_params = convert::mapped(&input.optional_params).map_err(invalid)?;
        let prepared = self.runtime.block_on(native_transform(
            &NativeRequest {
                model: &input.model,
                messages,
                optional_params,
                litellm_params: input.litellm_params.clone(),
            },
            &mut NativeSources {
                environment: &mut environment,
                catalogs: &catalogs,
                budgets: &self.budgets,
            },
        ));
        prepared.map_err(invalid)
    }
}
