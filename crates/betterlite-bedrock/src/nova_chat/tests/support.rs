use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::Verification;
use betterlite_config::types::ProxyTrust;
use betterlite_context::anthropic_catalog_lookup::RawCatalog;
use betterlite_context::anthropic_catalog_lookup::candidates::CandidatePatterns;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::{Resolve, UrlValidator};
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::catalog::{BundledCatalog, ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json as MetadataJson;
use betterlite_models::fetch_policy::response::FetchResponse;
use betterlite_models::fireworks_catalog_metadata::Catalog;
use betterlite_models::full_model_metadata::record::ModelInfo;
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
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::bedrock_images::remote::{RemoteImage, TRANSPORT_LIMITS};
use betterlite_prompts::message_content::Message;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use chrono::DateTime;
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio::runtime::{Builder, Runtime};

use crate::agentcore_stream::IdSource;
use crate::bedrock_model_capabilities::ModelInfoLookup;
use crate::converse_request::{
    ConverseMedia, ConverseMessageSettings, ConverseRequestInput, ConverseSettings,
    converse_request,
};
use crate::converse_response::wire::ModelResponse;
use crate::converse_usage::transform::ReasoningTokenCounter;
use crate::event_frames::beta::BetaHeader;
use crate::mantle_inputs::region::split_region_prefix;
use crate::model_routes::ParamMap;
use crate::nova_chat::error::NovaChatError;
use crate::nova_chat::invoke::{InvokeNovaOutcome, transform_request};

const NORMALIZERS: NameNormalizers = NameNormalizers {
    bedrock_base_model: get_bedrock_base_model,
    bedrock_routing_prefix: strip_bedrock_routing_prefix,
    mantle_region_split: split_region_prefix,
};

pub(super) const NOVA_PRO: &str = "amazon.nova-pro-v1:0";

pub(super) const MODEL_COST: &str = r#"{
    "amazon.nova-pro-v1:0": {
        "litellm_provider": "bedrock",
        "supports_prompt_caching": true,
        "supports_function_calling": true,
        "supports_tool_choice": true
    }
}"#;

pub(super) const USER_MESSAGES: &str = r#"[{"role": "user", "content": "hi"}]"#;

pub(super) fn params(source: &str) -> ParamMap {
    serde_json::from_str(source).expect("the parameters parse")
}

pub(super) fn object(source: &str) -> Object {
    serde_json::from_str(source).expect("the object parses")
}

pub(super) fn messages(source: &str) -> Vec<Message> {
    serde_json::from_str(source).expect("the messages parse")
}

pub(super) fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("the fixture JSON parses")
}

// The logging, reply and compatible-wire types carry decisions::Json, not the prompts copy.
pub(super) fn metadata_json(source: &str) -> MetadataJson {
    serde_json::from_str(source).expect("the fixture JSON parses")
}

pub(super) fn value<T>(value: &T) -> Json
where
    T: serde::Serialize,
{
    let encoded = serde_json::to_string(value).expect("the value renders");
    serde_json::from_str(&encoded).expect("the rendered value reads back")
}

pub(super) fn field<'value>(value: &'value Json, name: &str) -> Option<&'value Json> {
    value.as_object()?.get(name)
}

#[derive(Debug, thiserror::Error)]
#[error("the model is not mapped")]
pub(super) struct Unmapped;

pub(super) struct Answers(pub(super) BTreeMap<String, IndexMap<String, MetadataJson>>);

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

impl ModelInfoLookup for Answers {
    type Error = Unmapped;

    fn bedrock_model_info(
        &self,
        model: &str,
    ) -> impl Future<Output = Result<ModelInfo, Unmapped>> + Send {
        let answer = self.0.get(model).cloned();
        ready(answer.map(|fields| ModelInfo { fields }).ok_or(Unmapped))
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
    packaged: Arc<ModelCatalog>,
}

impl Fixture {
    pub(super) fn load(model_cost: &ParamMap) -> Self {
        let bytes = serde_json::to_vec(model_cost).expect("the model cost map renders");
        Self {
            entries: ModelCostMap::parse(&bytes).expect("the model cost map parses"),
            typed: ModelCatalog::parse(b"{}", UnknownFields::Warn)
                .expect("an empty catalog parses"),
            provider_catalog: Catalog::new(),
            hf: HfConfigClient {
                client: NoHfTransport,
                base_url: HF_CONFIG_BASE_URL.to_owned(),
            },
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            environment: BTreeMap::new(),
            patterns: CandidatePatterns::new().expect("the candidate patterns compile"),
            runtime: serde_json::from_slice(&bytes).expect("the raw catalog parses"),
            unlisted: RawCatalog::new(),
            packaged: BundledCatalog::default().get(),
        }
    }

    const fn catalogs(&self) -> CatalogSources<'_> {
        CatalogSources {
            patterns: &self.patterns,
            runtime: &self.runtime,
            packaged: &self.unlisted,
        }
    }

    fn packaged(&self) -> &ModelCatalog {
        self.packaged.as_ref()
    }

    pub(super) fn environment<'fixture>(
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

    pub(super) fn resolution(&self) -> ResolutionContext<'_> {
        let registry = JSONProviderRegistry::load();
        ResolutionContext {
            registry,
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
            catalog: &self.typed,
            known_model_names: &self.known_model_names,
            custom_providers: &self.custom_providers,
            environment: &self.environment,
        }
    }
}

pub(super) fn runtime() -> Runtime {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the test runtime builds")
}

pub(super) struct Call<'call> {
    pub(super) model: &'call str,
    pub(super) messages: &'call str,
    pub(super) optional_params: &'call str,
    pub(super) model_cost: &'call str,
}

impl<'call> Call<'call> {
    pub(super) const fn new(model: &'call str, optional_params: &'call str) -> Self {
        Self {
            model,
            messages: USER_MESSAGES,
            optional_params,
            model_cost: MODEL_COST,
        }
    }
}

pub(super) fn transform(call: &Call<'_>) -> Result<InvokeNovaOutcome, NovaChatError> {
    let model_cost = params(call.model_cost);
    let fixture = Fixture::load(&model_cost);
    let catalog_storage = fixture.catalogs();
    let mut optional_params = params(call.optional_params);
    let parsed_messages = messages(call.messages);
    let headers: BTreeMap<String, BetaHeader> = BTreeMap::new();
    let answers = Answers(BTreeMap::new());
    let mut keys = ModelCostKeys::default();
    let dynamic = NoDynamicMetadata;
    let resolution = fixture.resolution();
    let mut environment = fixture.environment(&resolution, &mut keys, &dynamic);
    let remote = remote();
    let new_id = || String::from("call_generated");
    let input = ConverseRequestInput {
        model: call.model,
        messages: &parsed_messages,
        optional_params: &mut optional_params,
        litellm_params: None,
        headers: Some(&headers),
        settings: ConverseSettings {
            global_drop: false,
            messages: ConverseMessageSettings {
                modify_params: false,
                neutralize_orphans: false,
            },
            request_metadata: None,
        },
        model_cost: &model_cost,
        catalogs: &catalog_storage,
        lookup: &answers,
        packaged: fixture.packaged(),
    };
    let media = ConverseMedia {
        remote: &remote,
        new_id: &new_id,
    };
    runtime().block_on(transform_request(input, media, &mut environment))
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Refusing;

#[async_trait::async_trait]
impl Resolve for Refusing {
    async fn resolve(&self, _host: &str, _port: u16) -> io::Result<Vec<SocketAddr>> {
        Err(io::Error::other("the resolver refuses every host"))
    }
}

fn remote() -> RemoteImage<Refusing> {
    remote_for(Refusing, &[], 1.0_f64)
}

fn remote_for<R>(resolver: R, allowed: &[&str], max_mb: f64) -> RemoteImage<R>
where
    R: Resolve,
{
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("the proxy policy resolves"),
    );
    let client = ProxiedClient::build(base_builder(), &policy, TRANSPORT_LIMITS)
        .expect("the proxied client builds");
    let hosts: Vec<String> = allowed.iter().map(|host| (*host).to_owned()).collect();
    let validator = UrlValidator::new(resolver, &hosts, &Verification::Enabled);
    RemoteImage {
        client,
        urls: UserUrlFetcher {
            validator,
            validation: true,
        },
        deadline: TRANSPORT_LIMITS.read_timeout,
        max_download_size_mb: max_mb,
    }
}

const INIT: &str = r#"{"model":"fixture-model","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const NOW: &str = "2026-01-01T00:00:05+00:00";

struct Silent;

impl LogSink for Silent {
    fn emit(&self, _record: LogRecord) {}
}

pub(super) struct SinkParts {
    redactor: Redactor,
    silent: Silent,
}

impl SinkParts {
    const fn new(redactor: Redactor) -> Self {
        Self {
            redactor,
            silent: Silent,
        }
    }

    pub(super) fn env(&self) -> PrePostEnv<'_> {
        let now = DateTime::parse_from_rfc3339(NOW).expect("the fixture clock parses");
        PrePostEnv {
            debug: DebugSettings::default(),
            globals: GlobalFlags::default(),
            decision: Decision {
                content: false,
                key_info: false,
            },
            redactor: &self.redactor,
            log: &self.silent,
            input_callbacks: &[],
            sentry: None,
            logger_fn: None,
            now,
        }
    }
}

// The conversion borrows the state mutably and the sink immutably, so the two live in separate fields.
pub(super) struct Logs {
    pub(super) state: LoggingState,
    pub(super) parts: SinkParts,
}

fn logging_state(redactor: &Redactor) -> LoggingState {
    LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).expect("the logging init parses"),
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("trace"),
        Duration::default(),
    )
    .expect("the logging state builds")
}

impl Logs {
    pub(super) fn new() -> Self {
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
        Self {
            state: logging_state(&redactor),
            parts: SinkParts::new(redactor),
        }
    }
}

pub(super) fn supplied(id: &str) -> ModelResponse {
    ModelResponse {
        id: id.to_owned(),
        created: 0,
        model: String::from("previous-model"),
        object: String::from("chat.completion"),
        system_fingerprint: None,
        choices: Vec::new(),
        usage: None,
        service_tier: None,
        trace: None,
    }
}

pub(super) fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

struct NoOidc;

impl OidcHttp for NoOidc {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: String::from("the fixture has no OIDC transport"),
        })
    }
}

impl AzureOidcTokens for NoOidc {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Ok(None)
    }
}

impl OidcClock for NoOidc {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

pub(super) fn secret_lookup() -> SecretLookup {
    SecretLookup {
        manager: None,
        oidc_http: Arc::new(NoOidc),
        azure_tokens: Arc::new(NoOidc),
        clock: Arc::new(NoOidc),
        oidc_cache: OidcTokenCache::default(),
    }
}

pub(super) struct FixedIds;

impl IdSource for FixedIds {
    fn next_id(&self) -> uuid::Uuid {
        uuid::Uuid::nil()
    }
}

pub(super) struct NoReasoning;

#[derive(Debug, thiserror::Error)]
#[error("the fixture counts no reasoning tokens")]
pub(super) struct NoReasoningError;

impl ReasoningTokenCounter for NoReasoning {
    type Failure = NoReasoningError;

    fn count_reasoning_tokens(&self, _text: &str) -> Result<i64, Self::Failure> {
        Ok(0)
    }
}
