use super::{
    Call, Observed, TIMEOUT, fixture::Fixture, media::Media, response::observe, services::Services,
};
use crate::anthropic_dispatch::{
    CallLogging,
    base::{Environment, Globals, Surface},
    completion::CompletionSettings,
    dispatch,
    fixtures::runtime::logging,
    native::{Native, Runtime, legacy::Legacy, preparation::Preparation},
    operations::{DispatchDeps, Operation},
};
use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::request::BillingHeader;
use betterlite_bedrock::mantle_inputs::region::split_region_prefix;
use betterlite_config::sections::anthropic_completion::OptionValues;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_core::cost::completion::lookup::NoHfConfig;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::{
    MetadataEnvironment, dynamic::NoDynamicMetadata, names::NameNormalizers,
};
use betterlite_models::resolution::ResolutionContext;
use betterlite_prompts::anthropic::assembly::PromptDeps;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use futures_util::future::LocalBoxFuture;
use std::collections::BTreeMap;
use tokio::time::timeout;

type Handler<'deps> =
    Native<'deps, ProxiedClient, NoDynamicMetadata, NoHfConfig, Media, Media, fn() -> String>;
type Capabilities<'deps> = CapabilityEnvironment<'deps, NoDynamicMetadata, NoHfConfig>;

impl Fixture {
    pub(super) fn invoke<'call>(
        &'call self,
        surface: Surface,
        call: Call,
        tag: ChatConfigTag,
    ) -> LocalBoxFuture<'call, Observed> {
        Box::pin(async move {
            let resolution = ResolutionContext::from(self);
            let mut keys = ModelCostKeys::default();
            let capabilities = CapabilityEnvironment {
                metadata: MetadataEnvironment {
                    model_cost: &self.map.entries,
                    vertex: &self.map.vertex,
                    generalizations: &self.generalizations,
                    resolution: &resolution,
                    normalizers: NameNormalizers {
                        bedrock_base_model: get_bedrock_base_model,
                        bedrock_routing_prefix: strip_bedrock_routing_prefix,
                        mantle_region_split: split_region_prefix,
                    },
                },
                keys: &mut keys,
                dynamic: &NoDynamicMetadata,
                hf: &self.hf,
                provider_catalog: &self.provider_catalog,
                provider_generation: 0,
            };
            self.complete(capabilities, surface, call, tag).await
        })
    }

    fn complete<'call>(
        &'call self,
        capabilities: Capabilities<'call>,
        surface: Surface,
        call: Call,
        tag: ChatConfigTag,
    ) -> LocalBoxFuture<'call, Observed> {
        Box::pin(async move {
            let mut handler = Native {
                runtime: Runtime::from(self),
                preparation: Preparation {
                    environment: capabilities,
                    catalogs: CatalogSources {
                        patterns: &self.patterns,
                        runtime: &self.source,
                        packaged: &self.source,
                    },
                    prompts: PromptDeps {
                        fetcher: &Media,
                        blocks: &Media,
                        new_id: &self.new_id,
                    },
                    catalog: &self.catalog,
                    defaults: &self.defaults,
                    max_tokens: 17,
                    billing: BillingHeader::Keep,
                    modify_params: false,
                    drop_params: false,
                },
                legacy: Legacy {
                    prompts: &self.prompts,
                    defaults: &OptionValues::default(),
                    body_limit: 0x0001_0000,
                },
            };
            self.dispatch(&mut handler, surface, call, tag).await
        })
    }

    fn dispatch<'call>(
        &'call self,
        handler: &'call mut Handler<'_>,
        surface: Surface,
        call: Call,
        tag: ChatConfigTag,
    ) -> LocalBoxFuture<'call, Observed> {
        Box::pin(async move {
            let services = Services::from(self);
            let globals = Globals::default();
            let environment = Environment::default();
            let completion = CompletionSettings {
                globals: &globals,
                environment: &environment,
                custom_prompt_dict: &BTreeMap::new(),
                disable_suffix: false,
                selection: Some(tag),
            };
            let mut deps = DispatchDeps {
                completion: &completion,
                handler,
                count: &services.count,
                batches: &services.batches,
                files: &services.files,
                file_client: &services.file_client,
            };
            logged_call(&mut deps, surface, call).await
        })
    }
}

fn logged_call<'call>(
    deps: &'call mut DispatchDeps<'_, ProxiedClient, Handler<'_>>,
    surface: Surface,
    call: Call,
) -> LocalBoxFuture<'call, Observed> {
    Box::pin(async move {
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
        let (mut state, env) = logging(&redactor).unwrap();
        let mut log = CallLogging {
            state: &mut state,
            env: &env,
        };
        let outcome = timeout(
            TIMEOUT,
            dispatch(
                Operation::Completion(surface, Box::new(call)),
                deps,
                &mut log,
            ),
        )
        .await
        .unwrap();
        let mut observed = observe(outcome).await;
        observed.logged = match state.model_call_details.get("original_response") {
            Some(Json::Text(text)) => text.clone(),
            _ => String::new(),
        };
        observed
    })
}
