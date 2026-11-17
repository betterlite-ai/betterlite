use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::fmt::Display;
use std::future::{Future, ready};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_anthropic::anthropic_capability_catalog::{CatalogSources, DropParams};
use betterlite_anthropic::headers::AnthropicHeaders;
use betterlite_anthropic::option_primitives::OptionDefaults;
use betterlite_anthropic::response::wire::ResponseEnvelope;
use betterlite_anthropic::response::{Logging, RawResponse, ResponseRequest};
use betterlite_anthropic::usage::ReasoningTokenCounter;
use betterlite_config::sections::anthropic_capability_catalog::Settings;
use betterlite_context::anthropic_catalog_lookup::RawCatalog;
use betterlite_context::anthropic_catalog_lookup::candidates::CandidatePatterns;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_core::api::anthropic_beta_config::matrix::{BetaMatrix, BetaNode};
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::catalog::{BundledCatalog, ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
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
use betterlite_prompts::anthropic::assembly::PromptDeps;
use betterlite_prompts::anthropic_results::tool_result::FileBlocks;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use http::HeaderMap;
use indexmap::IndexMap;
use tokio::runtime::{Builder, Runtime};

use crate::bedrock_model_capabilities::ModelInfoLookup;
use crate::claude_invoke::error::ClaudeInvokeError;
use crate::claude_invoke::params::{
    MapParamsError, MapParamsRequest, get_supported_openai_params, map_openai_params,
};
use crate::claude_invoke::request::{
    ClaudeInvokePrepared, ClaudeInvokeRequest, TransformSources, async_transform_request,
    transform_request,
};
use crate::claude_invoke::response::transform_response as claude_transform_response;
use crate::mantle_inputs::region::split_region_prefix;

const BUNDLED_BETA_CONFIG: &str = include_str!(
    "../../../../betterlite-core/src/api/anthropic_beta_config/anthropic_beta_headers_config.json"
);
const INIT: &str = r#"{"model":"claude-plain","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1","messages":[],"litellm_params":{},"request_logging":{}}"#;
const NOW: &str = "2026-01-01T00:00:05+00:00";
const NOW_UNIX: i64 = 1_767_225_600;

const NORMALIZERS: NameNormalizers = NameNormalizers {
    bedrock_base_model: get_bedrock_base_model,
    bedrock_routing_prefix: strip_bedrock_routing_prefix,
    mantle_region_split: split_region_prefix,
};

#[derive(Debug, thiserror::Error)]
#[error("the fixture is malformed: {message}")]
pub(super) struct FixtureError {
    message: String,
}

fn failed<E>(error: E) -> FixtureError
where
    E: Display,
{
    FixtureError {
        message: error.to_string(),
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the model is not mapped")]
pub(super) struct Unmapped;

#[derive(Default)]
pub(super) struct NoHfTransport;

impl ModelInfoClient for NoHfTransport {
    type Error = Unmapped;

    fn get(
        &self,
        _request: &ModelInfoRequest,
    ) -> impl Future<Output = Result<FetchResponse, Unmapped>> + Send {
        ready(Err(Unmapped))
    }
}

pub(super) struct MapLookup(BTreeMap<String, IndexMap<String, Json>>);

impl MapLookup {
    fn from_entries(entries: &IndexMap<String, IndexMap<String, Json>>) -> Self {
        let mut answers = BTreeMap::new();
        for (model, fields) in entries {
            answers.insert(model.clone(), fields.clone());
        }
        Self(answers)
    }
}

impl ModelInfoLookup for MapLookup {
    type Error = Unmapped;

    fn bedrock_model_info(
        &self,
        model: &str,
    ) -> impl Future<Output = Result<ModelInfo, Unmapped>> + Send {
        let answer = self
            .0
            .get(model)
            .or_else(|| self.0.get(&format!("bedrock/{model}")))
            .cloned();
        ready(answer.map(|fields| ModelInfo { fields }).ok_or(Unmapped))
    }
}

pub(super) struct NoFileBlocks;

impl FileBlocks for NoFileBlocks {
    type Failure = Infallible;

    fn file_block(&self, element: &Object) -> Result<Object, Infallible> {
        Ok(element.clone())
    }
}

pub(super) struct NoReasoning;

impl ReasoningTokenCounter for NoReasoning {
    type Failure = Infallible;

    fn count_reasoning_tokens(&self, _text: &str) -> Result<usize, Infallible> {
        Ok(0)
    }
}

fn record_call(fetcher_calls: &Mutex<Vec<String>>, url: &str) {
    fetcher_calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(url.to_owned());
}

fn text_pairs(entries: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut pairs = Vec::with_capacity(entries.len());
    for (name, value) in entries {
        pairs.push(((*name).to_owned(), (*value).to_owned()));
    }
    pairs
}

#[derive(Default)]
pub(super) struct MapFetcher {
    urls: BTreeMap<String, String>,
    calls: Mutex<Vec<String>>,
    transports: AtomicUsize,
}

impl MapFetcher {
    pub(super) fn new(entries: &[(&str, &str)]) -> Self {
        Self {
            urls: text_pairs(entries).into_iter().collect(),
            calls: Mutex::new(Vec::new()),
            transports: AtomicUsize::new(0),
        }
    }

    pub(super) fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn transport_attempts(&self) -> usize {
        self.transports.load(Ordering::Relaxed)
    }
}

impl RemoteMediaFetcher for MapFetcher {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        record_call(&self.calls, url);
        let answer = self
            .urls
            .get(url)
            .cloned()
            .ok_or_else(|| ImageFetchError::Attempts {
                url: url.to_owned(),
            });
        if answer.is_ok() {
            self.transports.fetch_add(1, Ordering::Relaxed);
        }
        ready(answer)
    }

    async fn convert_urls(&self, urls: &[String]) -> Result<Vec<String>, ImageFetchError> {
        let mut fetched = Vec::with_capacity(urls.len());
        for url in urls {
            fetched.push(self.convert_url(url).await?);
        }
        Ok(fetched)
    }
}

#[derive(Default)]
pub(super) struct RefusingFetcher {
    calls: Mutex<Vec<String>>,
    transports: AtomicUsize,
}

impl RefusingFetcher {
    pub(super) fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn transport_attempts(&self) -> usize {
        self.transports.load(Ordering::Relaxed)
    }
}

impl RemoteMediaFetcher for RefusingFetcher {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        record_call(&self.calls, url);
        ready(Err(ImageFetchError::Rejected {
            url: url.to_owned(),
        }))
    }

    async fn convert_urls(&self, urls: &[String]) -> Result<Vec<String>, ImageFetchError> {
        for url in urls {
            self.convert_url(url).await?;
        }
        Ok(Vec::new())
    }
}

struct Silent;

impl LogSink for Silent {
    fn emit(&self, _record: LogRecord) {}
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
    matrix: BetaMatrix,
    defaults: OptionDefaults,
}

impl Fixture {
    pub(super) fn new(
        model_cost: &IndexMap<String, IndexMap<String, Json>>,
    ) -> Result<Self, FixtureError> {
        let bytes = serde_json::to_vec(model_cost).map_err(failed)?;
        let mut bundled = BundledCatalog::default();
        Ok(Self {
            entries: ModelCostMap::parse(&bytes).map_err(failed)?,
            typed: ModelCatalog::parse(b"{}", UnknownFields::Warn).map_err(failed)?,
            packaged: bundled.get(),
            provider_catalog: Catalog::new(),
            hf: HfConfigClient {
                client: NoHfTransport,
                base_url: HF_CONFIG_BASE_URL.to_owned(),
            },
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            environment: BTreeMap::new(),
            patterns: CandidatePatterns::new().map_err(failed)?,
            runtime: serde_json::from_slice(&bytes).map_err(failed)?,
            unlisted: RawCatalog::new(),
            lookup: MapLookup::from_entries(model_cost),
            matrix: serde_json::from_str(BUNDLED_BETA_CONFIG).map_err(failed)?,
            defaults: OptionDefaults::default(),
        })
    }

    // The bundled bedrock matrix maps the mcp beta to null, so this keeps it to observe the header.
    pub(super) fn keep_mcp_betas(&mut self) {
        let Some(BetaNode::Map(bedrock)) = self.matrix.0.get_mut("bedrock") else {
            panic!("the bundled beta matrix maps bedrock");
        };
        bedrock.insert(
            "mcp-client-2025-04-04".to_owned(),
            BetaNode::Text("mcp-client-2025-04-04".to_owned()),
        );
    }

    pub(super) fn request<'request>(
        &'request self,
        model: &'request str,
        messages: Vec<Json>,
        optional_params: IndexMap<String, Json>,
        headers: AnthropicHeaders,
    ) -> ClaudeInvokeRequest<'request> {
        ClaudeInvokeRequest {
            model,
            messages,
            optional_params,
            litellm_params: IndexMap::new(),
            headers,
            defaults: &self.defaults,
            catalog: &self.typed,
            default_chat_max_tokens: 4096,
            modify_params: false,
            global_drop_params: false,
        }
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

#[derive(Clone, Copy)]
pub(super) enum Mode {
    Sync,
    Async,
}

pub(super) struct Session<'fixture> {
    fixture: &'fixture Fixture,
    resolution: ResolutionContext<'fixture>,
    keys: ModelCostKeys,
    dynamic: NoDynamicMetadata,
    budgets: Settings,
    runtime: Runtime,
}

impl<'fixture> Session<'fixture> {
    pub(super) fn new(fixture: &'fixture Fixture) -> Result<Self, FixtureError> {
        Ok(Self {
            fixture,
            resolution: fixture.resolution(),
            keys: ModelCostKeys::default(),
            dynamic: NoDynamicMetadata,
            budgets: Settings::default(),
            runtime: Builder::new_current_thread().build().map_err(failed)?,
        })
    }

    pub(super) fn transform<Fetcher>(
        &mut self,
        request: &ClaudeInvokeRequest<'_>,
        fetcher: &Fetcher,
        mode: Mode,
    ) -> Result<ClaudeInvokePrepared, ClaudeInvokeError<Infallible>>
    where
        Fetcher: RemoteMediaFetcher + Sync,
    {
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
        match mode {
            Mode::Sync => self
                .runtime
                .block_on(transform_request(request, &mut sources)),
            Mode::Async => self
                .runtime
                .block_on(async_transform_request(request, &mut sources)),
        }
    }

    pub(super) fn map_params(
        &mut self,
        model: &str,
        drop_params: bool,
        non_default_params: &IndexMap<String, Json>,
        optional_params: &mut IndexMap<String, Json>,
    ) -> Result<(), MapParamsError> {
        let catalogs = self.fixture.catalogs();
        let request = MapParamsRequest {
            model,
            drop: DropParams {
                global: false,
                call: drop_params,
            },
            budgets: &self.budgets,
            default_max_tokens: 4096,
            catalogs: &catalogs,
            packaged: self.fixture.packaged.as_ref(),
            lookup: &self.fixture.lookup,
        };
        let mut environment =
            self.fixture
                .capability_environment(&self.resolution, &mut self.keys, &self.dynamic);
        self.runtime.block_on(map_openai_params(
            &request,
            non_default_params,
            optional_params,
            &mut environment,
        ))
    }

    pub(super) fn supported(&mut self, model: &str) -> Vec<&'static str> {
        let catalogs = self.fixture.catalogs();
        let mut environment =
            self.fixture
                .capability_environment(&self.resolution, &mut self.keys, &self.dynamic);
        self.runtime.block_on(get_supported_openai_params(
            model,
            &mut environment,
            &catalogs,
        ))
    }
}

pub(super) fn response(raw: &str, messages: &[Json]) -> ResponseEnvelope {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
    let mut state = logging_state(&redactor);
    let sink = Silent;
    let env = pre_post_env(&redactor, &sink);
    let mut envelope = ResponseEnvelope::new("generated".to_owned());
    let headers = HeaderMap::new();
    let raw_response = RawResponse {
        status: 200,
        headers: &headers,
        body: raw.as_bytes(),
    };
    let optional_params = IndexMap::new();
    let litellm_params = IndexMap::new();
    let request_data = IndexMap::new();
    let request = ResponseRequest {
        messages,
        optional_params: &optional_params,
        litellm_params: &litellm_params,
        request_data: &request_data,
        json_mode: None,
        now: NOW_UNIX,
        disable_add_prefix_to_prompt: false,
    };
    claude_transform_response(
        &mut envelope,
        &raw_response,
        &request,
        Logging {
            state: &mut state,
            env: &env,
        },
        &NoReasoning,
    )
    .expect("the response transforms");
    envelope
}

fn logging_state(redactor: &Redactor) -> LoggingState {
    let mut context = CorrelationContext::default();
    LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).expect("the logging init parses"),
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        redactor,
        || "generated".to_owned(),
        Duration::default,
    )
    .expect("the logging state builds")
}

fn pre_post_env<'env>(redactor: &'env Redactor, sink: &'env Silent) -> PrePostEnv<'env> {
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
        now: DateTime::<FixedOffset>::parse_from_rfc3339(NOW).expect("the clock parses"),
    }
}

pub(super) fn transformed<Fetcher>(
    session: &mut Session<'_>,
    request: &ClaudeInvokeRequest<'_>,
    fetcher: &Fetcher,
    mode: Mode,
) -> Result<Json, ClaudeInvokeError<Infallible>>
where
    Fetcher: RemoteMediaFetcher + Sync,
{
    session
        .transform(request, fetcher, mode)
        .map(|prepared| body_json(&prepared))
}

pub(super) fn plain_body<Fetcher>(
    fixture: &Fixture,
    session: &mut Session<'_>,
    messages: Vec<Json>,
    fetcher: &Fetcher,
    mode: Mode,
) -> Result<Json, ClaudeInvokeError<Infallible>>
where
    Fetcher: RemoteMediaFetcher + Sync,
{
    let request = fixture.request(
        "claude-plain",
        messages,
        map_of(r#"{"max_tokens": 64}"#),
        AnthropicHeaders::default(),
    );
    transformed(session, &request, fetcher, mode)
}

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture JSON parses")
}

pub(super) fn field<'value>(value: &'value Json, name: &str) -> Option<&'value Json> {
    match value {
        Json::Object(fields) => fields.get(name),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

pub(super) fn at<'value>(value: &'value Json, path: &[&str]) -> &'value Json {
    let mut current = value;
    for step in path {
        current = match current {
            Json::Object(fields) => fields.get(*step).expect("the object field exists"),
            Json::List(items) => {
                let index: usize = step.parse().expect("a list step is an index");
                items.get(index).expect("the list index exists")
            }
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => {
                panic!("the path meets a scalar")
            }
        };
    }
    current
}

pub(super) fn prompt_message() -> Vec<Json> {
    messages_of(r#"[{"role": "user", "content": "hello"}]"#)
}

pub(super) fn map_of(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).expect("the fixture params parse")
}

pub(super) fn model_cost_of(text: &str) -> IndexMap<String, IndexMap<String, Json>> {
    serde_json::from_str(text).expect("the fixture model cost parses")
}

pub(super) fn messages_of(text: &str) -> Vec<Json> {
    serde_json::from_str(text).expect("the fixture messages parse")
}

pub(super) fn body_json(prepared: &ClaudeInvokePrepared) -> Json {
    serde_json::from_value(serde_json::to_value(&prepared.body).expect("the body serializes"))
        .expect("the body round-trips as JSON")
}

pub(super) fn headers_of(entries: &[(&str, &str)]) -> AnthropicHeaders {
    let mut headers = AnthropicHeaders::default();
    headers.values.extend(text_pairs(entries));
    headers
}
