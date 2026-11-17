use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_anthropic::anthropic_capability_catalog::{CatalogSources, DropParams};
use betterlite_anthropic::headers::AnthropicHeaders;
use betterlite_anthropic::option_primitives::OptionDefaults;
use betterlite_anthropic::params::error::ParamsError;
use betterlite_anthropic::params::{DEFAULT_MAX_TOKENS, ParamsRequest};
use betterlite_anthropic::request::{AnthropicRequest, BillingHeader, PreparedRequest};
use betterlite_anthropic::response::wire::ResponseEnvelope;
use betterlite_anthropic::response::{RawResponse, ResponseRequest};
use betterlite_anthropic::usage::ReasoningTokenCounter;
use betterlite_bedrock::mantle_inputs::region::split_region_prefix;
use betterlite_config::sections::anthropic_capability_catalog::Settings;
use betterlite_context::anthropic_catalog_lookup::RawCatalog;
use betterlite_context::anthropic_catalog_lookup::candidates::CandidatePatterns;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
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
use betterlite_prompts::anthropic::assembly::PromptDeps;
use betterlite_prompts::anthropic_results::tool_result::FileBlocks;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use http::HeaderMap;
use indexmap::IndexMap;
use tokio::runtime::Builder;

use crate::vertex_claude_chat::PROVIDER;
use crate::vertex_claude_chat::error::ClaudeChatError;
use crate::vertex_claude_chat::params::map_openai_params;
use crate::vertex_claude_chat::request::prepare;
use crate::vertex_claude_chat::response::{TransformCall, transform_response};

// Models that resolve only under vertex_ai carry the prefix, so a missed provider override loses the capability.
pub(crate) const MODELS: &str = r#"{
    "claude-haiku-4-5": {"litellm_provider": "vertex_ai", "supports_speed": true},
    "claude-3-sonnet-20240229": {"litellm_provider": "vertex_ai", "supports_forced_tool_use": false},
    "vertex_ai/claude-opus-4-5": {"litellm_provider": "vertex_ai", "supports_output_config": true},
    "vertex_ai/claude-sonnet-4-5": {"litellm_provider": "vertex_ai", "supports_adaptive_thinking": true},
    "vertex_ai/claude-sonnet-4-6": {"litellm_provider": "vertex_ai", "supports_native_structured_output": true, "supports_adaptive_thinking": true}
}"#;

pub(crate) const SIMPLE: &str = r#"[{"role":"user","content":"hi"}]"#;
pub(crate) const ID: &str = "generated-id";

const NORMALIZERS: NameNormalizers = NameNormalizers {
    bedrock_base_model: get_bedrock_base_model,
    bedrock_routing_prefix: strip_bedrock_routing_prefix,
    mantle_region_split: split_region_prefix,
};

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

#[derive(Default)]
pub(crate) struct RefusingFetcher {
    calls: AtomicUsize,
}

impl RefusingFetcher {
    pub(crate) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

fn refused(url: &str) -> ImageFetchError {
    ImageFetchError::Disabled {
        url: url.to_owned(),
    }
}

impl RemoteMediaFetcher for RefusingFetcher {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        ready(Err(refused(url)))
    }

    async fn convert_urls(&self, urls: &[String]) -> Result<Vec<String>, ImageFetchError> {
        let first = urls.first().map_or("", String::as_str);
        self.convert_url(first).await?;
        Ok(Vec::new())
    }
}

pub(crate) const INLINED_DATA_URL: &str = "data:image/png;base64,QUJD";

#[derive(Default)]
pub(crate) struct InliningFetcher {
    urls: Mutex<Vec<String>>,
}

impl InliningFetcher {
    pub(crate) fn urls(&self) -> Vec<String> {
        self.urls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl RemoteMediaFetcher for InliningFetcher {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        self.urls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(url.to_owned());
        ready(Ok(INLINED_DATA_URL.to_owned()))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        let mut inlined = Vec::with_capacity(urls.len());
        for url in urls {
            self.urls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(url.clone());
            inlined.push(INLINED_DATA_URL.to_owned());
        }
        ready(Ok(inlined))
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

pub(crate) struct Fixture {
    map: ModelCostMap,
    catalog: ModelCatalog,
    generalizations: CatalogGeneralizations,
    provider_catalog: Catalog,
    hf: HfConfigClient<NoHfTransport>,
    known_model_names: BTreeSet<String>,
    custom_providers: BTreeSet<String>,
    environment: BTreeMap<String, String>,
    patterns: CandidatePatterns,
    runtime: RawCatalog,
    packaged: RawCatalog,
}

impl Fixture {
    pub(crate) fn new(model_cost: &str) -> Self {
        Self::with_packaged(model_cost, "{}")
    }

    pub(crate) fn with_packaged(model_cost: &str, packaged: &str) -> Self {
        Self {
            map: ModelCostMap::parse(model_cost.as_bytes()).expect("model cost map"),
            // every case names its provider, so resolution never reads the typed catalog
            catalog: ModelCatalog::parse(b"{}", UnknownFields::Warn).expect("empty catalog"),
            generalizations: CatalogGeneralizations::default(),
            provider_catalog: Catalog::new(),
            hf: HfConfigClient {
                client: NoHfTransport,
                base_url: HF_CONFIG_BASE_URL.to_owned(),
            },
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            environment: BTreeMap::new(),
            patterns: CandidatePatterns::new().expect("candidate patterns"),
            runtime: serde_json::from_str(model_cost).expect("runtime catalog"),
            packaged: serde_json::from_str(packaged).expect("packaged catalog"),
        }
    }

    pub(crate) fn catalog(&self) -> &ModelCatalog {
        &self.catalog
    }

    pub(crate) fn catalogs(&self) -> CatalogSources<'_> {
        CatalogSources {
            patterns: &self.patterns,
            runtime: &self.runtime,
            packaged: &self.packaged,
        }
    }

    fn resolution(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.known_model_names,
            custom_providers: &self.custom_providers,
            environment: &self.environment,
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }

    fn environment<'fixture>(
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
}

pub(crate) struct Session<'fixture> {
    fixture: &'fixture Fixture,
    resolution: ResolutionContext<'fixture>,
    keys: ModelCostKeys,
    dynamic: NoDynamicMetadata,
}

impl<'fixture> Session<'fixture> {
    pub(crate) fn new(fixture: &'fixture Fixture) -> Self {
        Self {
            fixture,
            resolution: fixture.resolution(),
            keys: ModelCostKeys::default(),
            dynamic: NoDynamicMetadata,
        }
    }

    pub(crate) fn environment(
        &mut self,
    ) -> CapabilityEnvironment<'_, NoDynamicMetadata, NoHfTransport> {
        self.fixture
            .environment(&self.resolution, &mut self.keys, &self.dynamic)
    }
}

pub(crate) struct Scenario {
    pub(crate) model: String,
    messages: Vec<Json>,
    optional_params: IndexMap<String, Json>,
    headers: IndexMap<String, String>,
    defaults: OptionDefaults,
    billing_header: BillingHeader,
}

impl Scenario {
    pub(crate) fn new(model: &str, source: &str) -> Self {
        Self {
            model: model.to_owned(),
            messages: messages(source),
            optional_params: IndexMap::new(),
            headers: IndexMap::new(),
            defaults: OptionDefaults::default(),
            billing_header: BillingHeader::Keep,
        }
    }

    pub(crate) fn optional(mut self, source: &str) -> Self {
        self.optional_params = fields(source);
        self
    }

    pub(crate) fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.insert(name.to_owned(), value.to_owned());
        self
    }

    pub(crate) fn run_with<F>(&self, fetcher: &F) -> Outcome
    where
        F: RemoteMediaFetcher + Sync,
    {
        let fixture = Fixture::new(MODELS);
        let mut session = Session::new(&fixture);
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let result = runtime.block_on(self.prepare(&mut session, &fixture, fetcher));
        Outcome::new(result)
    }

    async fn prepare<F>(
        &self,
        session: &mut Session<'_>,
        fixture: &Fixture,
        fetcher: &F,
    ) -> Result<PreparedRequest, ClaudeChatError<NoFileError>>
    where
        F: RemoteMediaFetcher + Sync,
    {
        let request = AnthropicRequest {
            model: &self.model,
            custom_llm_provider: None,
            messages: self.messages.clone(),
            optional_params: self.optional_params.clone(),
            litellm_params: IndexMap::new(),
            headers: AnthropicHeaders {
                values: self.headers.clone(),
            },
            defaults: &self.defaults,
            catalog: fixture.catalog(),
            default_chat_max_tokens: DEFAULT_MAX_TOKENS,
            billing_header: self.billing_header,
            modify_params: false,
            global_drop_params: false,
        };
        let blocks = NoFileBlocks;
        let new_id = || ID.to_owned();
        let prompts = PromptDeps {
            fetcher,
            blocks: &blocks,
            new_id: &new_id,
        };
        prepare(
            request,
            &mut session.environment(),
            &fixture.catalogs(),
            &prompts,
        )
        .await
    }
}

pub(crate) fn map(
    model: &str,
    requested: &str,
    initial: &str,
    drop: DropParams,
) -> Result<IndexMap<String, Json>, ParamsError> {
    map_with_packaged(model, requested, initial, drop, "{}")
}

pub(crate) fn map_with_packaged(
    model: &str,
    requested: &str,
    initial: &str,
    drop: DropParams,
    packaged: &str,
) -> Result<IndexMap<String, Json>, ParamsError> {
    let fixture = Fixture::with_packaged(MODELS, packaged);
    let mut session = Session::new(&fixture);
    let catalogs = fixture.catalogs();
    let budgets = Settings::default();
    let request = ParamsRequest {
        model,
        provider: PROVIDER,
        drop,
        budgets: &budgets,
        default_max_tokens: DEFAULT_MAX_TOKENS,
        catalogs: &catalogs,
    };
    let non_default = fields(requested);
    let mut optional = fields(initial);
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(map_openai_params(
        &request,
        &non_default,
        &mut optional,
        &mut session.environment(),
    ))?;
    Ok(optional)
}

const LOGGING_INIT: &str = r#"{"model":"claude-sonnet-4-5","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const LOGGING_NOW: &str = "2026-01-01T00:00:05+00:00";
pub(crate) const RESPONSE: &str = r#"{"model":"claude-3-sonnet-20240229","content":[{"type":"text","text":"world"}],"stop_reason":"end_turn","usage":{"input_tokens":3,"output_tokens":1}}"#;

struct NoReasoningTokens;

impl ReasoningTokenCounter for NoReasoningTokens {
    type Failure = Infallible;

    fn count_reasoning_tokens(&self, _text: &str) -> Result<usize, Self::Failure> {
        Ok(0)
    }
}

struct Silent;

impl LogSink for Silent {
    fn emit(&self, _record: LogRecord) {}
}

fn logging_state(redactor: &Redactor) -> LoggingState {
    let init = serde_json::from_str::<LoggingInit>(LOGGING_INIT).expect("logging init");
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("generated"),
        Duration::default,
    )
    .expect("logging state")
}

fn logging_env<'env>(redactor: &'env Redactor, sink: &'env Silent) -> PrePostEnv<'env> {
    let now = DateTime::<FixedOffset>::parse_from_rfc3339(LOGGING_NOW).expect("clock");
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now,
    }
}

pub(crate) fn transform_response_body(model: &str, body: &str) -> ResponseEnvelope {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("redactor");
    let sink = Silent;
    let mut state = logging_state(&redactor);
    let env = logging_env(&redactor, &sink);
    let messages = messages(SIMPLE);
    let optional_params = IndexMap::new();
    let litellm_params = IndexMap::new();
    let request_data = IndexMap::new();
    let request = ResponseRequest {
        messages: &messages,
        optional_params: &optional_params,
        litellm_params: &litellm_params,
        request_data: &request_data,
        json_mode: None,
        now: 0,
        disable_add_prefix_to_prompt: false,
    };
    let headers = HeaderMap::new();
    let raw = RawResponse {
        status: 200,
        headers: &headers,
        body: body.as_bytes(),
    };
    let counter = NoReasoningTokens;
    let mut envelope = ResponseEnvelope::new(ID.to_owned());
    transform_response(TransformCall {
        model,
        envelope: &mut envelope,
        raw: &raw,
        request: &request,
        state: &mut state,
        env: &env,
        counter: &counter,
    })
    .expect("the response transforms");
    envelope
}

pub(crate) struct Outcome {
    pub(crate) body: IndexMap<String, Json>,
    pub(crate) headers: IndexMap<String, String>,
    failure: Option<(Option<u16>, String)>,
}

impl Outcome {
    fn new(result: Result<PreparedRequest, ClaudeChatError<NoFileError>>) -> Self {
        match result {
            Ok(prepared) => Self {
                body: prepared.body,
                headers: prepared.headers.values,
                failure: None,
            },
            Err(error) => Self {
                body: IndexMap::new(),
                headers: IndexMap::new(),
                failure: Some((error.status_code(), error.to_string())),
            },
        }
    }

    pub(crate) fn ok(self) -> Self {
        assert!(self.failure.is_none(), "request failed: {:?}", self.failure);
        self
    }

    pub(crate) fn failed(mut self) -> (Option<u16>, String) {
        self.failure.take().expect("the request raises")
    }

    pub(crate) fn get(&self, key: &str) -> Option<&Json> {
        self.body.get(key)
    }

    pub(crate) fn beta(&self) -> Option<&str> {
        self.headers.get("anthropic-beta").map(String::as_str)
    }
}

pub(crate) fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("case JSON")
}

pub(crate) fn fields(source: &str) -> IndexMap<String, Json> {
    match json(source) {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            IndexMap::new()
        }
    }
}

pub(crate) fn messages(source: &str) -> Vec<Json> {
    match json(source) {
        Json::List(items) => items,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
            Vec::new()
        }
    }
}

// Both sides read as the crate's own JSON tree, whose object equality ignores key order
pub(crate) fn expect_field(outcome: &Outcome, key: &str, expected: &str) {
    let actual = outcome
        .get(key)
        .unwrap_or_else(|| panic!("the body has no {key}"));
    assert_eq!(*actual, json(expected), "{key}");
}

pub(crate) fn expect_absent(outcome: &Outcome, key: &str) {
    assert!(
        outcome.get(key).is_none(),
        "the body still carries {key}: {:?}",
        outcome.get(key)
    );
}
