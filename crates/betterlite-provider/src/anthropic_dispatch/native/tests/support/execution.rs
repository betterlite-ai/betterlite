use super::{
    Invocation, Observed, TIMEOUT, fixture::Fixture, media::Media, response::observe,
    services::Services,
};
use crate::anthropic_dispatch::{
    dispatch,
    fixtures::runtime::logging,
    native::{Native, Runtime, legacy::Legacy, preparation::Preparation},
    operations::{DispatchDeps, Operation},
};
use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::anthropic_http_stream::call::{StreamLogger, StreamPostCall};
use betterlite_anthropic::request::BillingHeader;
use betterlite_anthropic::stream_state::state::UuidV4;
use betterlite_bedrock::mantle_inputs::region::split_region_prefix;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_core::cost::completion::lookup::NoHfConfig;
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::{
    MetadataEnvironment, dynamic::NoDynamicMetadata, names::NameNormalizers,
};
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_prompts::anthropic::assembly::PromptDeps;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use tokio::time::timeout;

type Handler<'deps> =
    Native<'deps, ProxiedClient, NoDynamicMetadata, NoHfConfig, Media, Media, fn() -> String>;
type Capabilities<'deps> = CapabilityEnvironment<'deps, NoDynamicMetadata, NoHfConfig>;

pub(super) async fn invoke<'call>(
    fixture: &'call Fixture,
    invocation: Invocation<'call>,
) -> Observed {
    let resolution = ResolutionContext {
        catalog: &fixture.catalog,
        known_model_names: &fixture.names,
        custom_providers: &fixture.names,
        environment: &fixture.environment,
        registry: JSONProviderRegistry::load(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let mut keys = ModelCostKeys::default();
    let capabilities = CapabilityEnvironment {
        metadata: MetadataEnvironment {
            model_cost: &fixture.map.entries,
            vertex: &fixture.map.vertex,
            generalizations: &fixture.generalizations,
            resolution: &resolution,
            normalizers: NameNormalizers {
                bedrock_base_model: get_bedrock_base_model,
                bedrock_routing_prefix: strip_bedrock_routing_prefix,
                mantle_region_split: split_region_prefix,
            },
        },
        keys: &mut keys,
        dynamic: &NoDynamicMetadata,
        hf: &fixture.hf,
        provider_catalog: &fixture.provider_catalog,
        provider_generation: 0,
    };
    let mut handler = Handler::from((fixture, capabilities));
    invocation.dispatch(&mut handler, &fixture.services).await
}

impl<'deps> From<(&'deps Fixture, Capabilities<'deps>)> for Handler<'deps> {
    fn from((fixture, capabilities): (&'deps Fixture, Capabilities<'deps>)) -> Self {
        Self {
            runtime: Runtime {
                pooled: &fixture.client,
                settings: &fixture.settings,
                matrix: &fixture.matrix,
                stream_logger: &Log,
                ids: &UuidV4,
                now: fixture.now,
                max_event_bytes: 0x0001_0000,
                max_error_body_bytes: 0x0001_0000,
                disable_add_prefix_to_prompt: false,
                effort_used: false,
            },
            preparation: Preparation {
                environment: capabilities,
                catalogs: CatalogSources {
                    patterns: &fixture.patterns,
                    runtime: &fixture.source,
                    packaged: &fixture.source,
                },
                prompts: PromptDeps {
                    fetcher: &Media,
                    blocks: &Media,
                    new_id: &fixture.new_id,
                },
                catalog: &fixture.catalog,
                defaults: &fixture.defaults,
                max_tokens: 17,
                billing: BillingHeader::Keep,
                modify_params: false,
                drop_params: false,
            },
            legacy: Legacy {
                prompts: &fixture.prompts,
                defaults: &fixture.legacy_defaults,
                body_limit: 0x0001_0000,
            },
        }
    }
}

struct Log;
impl StreamLogger for Log {
    fn post_call(&self, _call: StreamPostCall<'_>) {}
}

impl<'deps> Invocation<'deps> {
    async fn dispatch<'call>(
        self,
        handler: &'call mut Handler<'_>,
        services: &'call Services,
    ) -> Observed
    where
        'deps: 'call,
    {
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
        let (mut state, env) = logging(&redactor).unwrap();
        let mut log = LogContext {
            state: &mut state,
            env: &env,
        };
        let mut deps = DispatchDeps {
            completion: &self.completion,
            handler,
            count: &services.count,
            batches: &services.batches,
            files: &services.files,
            file_client: &services.file_client,
        };
        let outcome = timeout(
            TIMEOUT,
            dispatch(
                Operation::Completion(self.surface, Box::new(self.call)),
                &mut deps,
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
    }
}
