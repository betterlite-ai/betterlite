use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::headers::AnthropicHeaders;
use betterlite_anthropic::option_primitives::OptionDefaults;
use betterlite_config::sections::anthropic_capability_catalog::Settings;
use betterlite_context::anthropic_catalog_lookup::RawCatalog;
use betterlite_context::anthropic_catalog_lookup::candidates::CandidatePatterns;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_core::api::anthropic_beta_config::matrix::BetaMatrix;
use betterlite_core::api::messages_stream_logging::StreamDeps;
use betterlite_core::api::messages_stream_logging::seams::{
    ChunkProcessorCall, LoggingFuture, ReportedFailure, RouteStreamingLogging, StreamSeams,
};
use betterlite_core::api::messages_stream_logging::slots::StreamSlots;
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_logging::passthrough_logging_state::logger::PassThroughEndpointLogging;
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamSuccessCall, StreamSuccessDispatch,
};
use betterlite_logging::passthrough_stream_logging::types::EndpointType;
use betterlite_logging::worker::{LoggingWorker, WorkerSettings};
use betterlite_models::catalog::{BundledCatalog, ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json as ParamJson;
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
use betterlite_prompts::arguments::json::{Json as BodyJson, Object};
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use tokio::runtime::{Builder, Runtime};

use crate::bedrock_model_capabilities::ModelInfoLookup;
use crate::claude_invoke::error::ClaudeInvokeError;
use crate::claude_invoke::request::{ClaudeInvokeRequest, TransformSources as ChatSources};
use crate::event_frames::beta::BetaHeader;
use crate::invoke_primitives::config::Params as InvokeParams;
use crate::mantle_chat::error::MantleMessagesError;
use crate::mantle_chat::messages::transform_request as messages_transform_request;
use crate::mantle_chat::request::{
    async_transform_request, transform_request as chat_transform_request,
};
use crate::mantle_inputs::region::split_region_prefix;
use crate::messages::transform::{MessagesRequest, TransformSources as MessagesSources};
use crate::model_routes::ParamMap;

const BUNDLED_BETA_CONFIG: &str = include_str!(
    "../../../../../crates/betterlite-core/src/api/anthropic_beta_config/anthropic_beta_headers_config.json"
);

const NORMALIZERS: NameNormalizers = NameNormalizers {
    bedrock_base_model: get_bedrock_base_model,
    bedrock_routing_prefix: strip_bedrock_routing_prefix,
    mantle_region_split: split_region_prefix,
};

// The same map the sibling Bedrock tests use: a plain chat model, tool search, adaptive thinking, effort tiers and
// native structured output, so the capability branches of the base builder stay reachable.
pub(super) const MODEL_COST: &str = r#"{
  "bedrock/fixture-plain": {"litellm_provider": "bedrock", "mode": "chat", "supports_mid_conversation_system": false},
  "fixture-plain": {"litellm_provider": "bedrock", "mode": "chat", "supports_mid_conversation_system": false},
  "bedrock/fixture-tool-search": {"litellm_provider": "bedrock", "supports_tool_search": true},
  "fixture-tool-search": {"litellm_provider": "bedrock", "supports_tool_search": true},
  "bedrock/fixture-effort-tier": {"litellm_provider": "bedrock", "supports_xhigh_reasoning_effort": true, "bedrock_output_config_effort_ceiling": "high"},
  "fixture-effort-tier": {"litellm_provider": "bedrock", "supports_xhigh_reasoning_effort": true, "bedrock_output_config_effort_ceiling": "high"}
}"#;

pub(super) fn js(text: &str) -> BodyJson {
    serde_json::from_str(text).expect("the fixture JSON parses")
}

pub(super) fn obj(text: &str) -> Object {
    let BodyJson::Object(object) = js(text) else {
        panic!("the fixture JSON is an object");
    };
    object
}

pub(super) fn list(text: &str) -> Vec<BodyJson> {
    let BodyJson::List(items) = js(text) else {
        panic!("the fixture JSON is a list");
    };
    items
}

pub(super) fn map_of(text: &str) -> IndexMap<String, ParamJson> {
    serde_json::from_str(text).expect("the fixture params parse")
}

pub(super) fn messages_of(text: &str) -> Vec<ParamJson> {
    serde_json::from_str(text).expect("the fixture messages parse")
}

pub(super) fn runtime() -> Runtime {
    Builder::new_current_thread()
        .build()
        .expect("the test runtime builds")
}

pub(super) fn invoke_params(text: &str) -> InvokeParams {
    serde_json::from_str(text).expect("the fixture invoke params parse")
}

pub(super) fn headers_of(text: &str) -> BTreeMap<String, BetaHeader> {
    serde_json::from_str(text).expect("the fixture beta headers parse")
}

// The stream factory takes the contract JSON map, so a request body reaches it through the same round-trip.
pub(super) fn contract_of(body: &Object) -> IndexMap<String, ParamJson> {
    let text = serde_json::to_string(body).expect("the body serializes");
    serde_json::from_str(&text).expect("the body round-trips")
}

#[derive(Debug, thiserror::Error)]
#[error("the metadata lookup must not fetch a Hugging Face config")]
pub(super) struct NoHfError;

#[derive(Default)]
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

#[derive(Debug, thiserror::Error)]
#[error("the model is not mapped")]
pub(super) struct Unmapped;

pub(super) struct MapLookup(BTreeMap<String, IndexMap<String, ParamJson>>);

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

fn record_call(calls: &Mutex<Vec<String>>, url: &str) {
    calls
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
    budgets: Settings,
    model_cost: ParamMap,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let bytes = MODEL_COST.as_bytes();
        let entries = ModelCostMap::parse(bytes).expect("the model cost map parses");
        let answers: IndexMap<String, IndexMap<String, ParamJson>> =
            serde_json::from_str(MODEL_COST).expect("the lookup answers parse");
        Self {
            entries,
            typed: ModelCatalog::parse(b"{}", UnknownFields::Warn).expect("an empty catalog"),
            packaged: BundledCatalog::default().get(),
            provider_catalog: Catalog::new(),
            hf: HfConfigClient {
                client: NoHfTransport,
                base_url: HF_CONFIG_BASE_URL.to_owned(),
            },
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            environment: BTreeMap::new(),
            patterns: CandidatePatterns::new().expect("the candidate patterns compile"),
            runtime: serde_json::from_slice(bytes).expect("the raw catalog parses"),
            unlisted: RawCatalog::new(),
            lookup: MapLookup(answers.into_iter().collect()),
            matrix: serde_json::from_str(BUNDLED_BETA_CONFIG).expect("the bundled matrix parses"),
            defaults: OptionDefaults::default(),
            budgets: Settings::default(),
            model_cost: serde_json::from_str(MODEL_COST).expect("the model cost params parse"),
        }
    }

    pub(super) fn chat_request<'request>(
        &'request self,
        model: &'request str,
        messages: Vec<ParamJson>,
        optional_params: IndexMap<String, ParamJson>,
    ) -> ClaudeInvokeRequest<'request> {
        ClaudeInvokeRequest {
            model,
            messages,
            optional_params,
            litellm_params: IndexMap::new(),
            headers: AnthropicHeaders::default(),
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

pub(super) struct MessagesCall {
    pub(super) model: String,
    pub(super) messages: Vec<BodyJson>,
    pub(super) optional_params: Object,
    pub(super) header_values: BTreeMap<String, BetaHeader>,
}

pub(super) fn messages_call(
    model: &str,
    messages: &str,
    params: &str,
    header_values: &str,
) -> MessagesCall {
    MessagesCall {
        model: model.to_owned(),
        messages: list(messages),
        optional_params: obj(params),
        header_values: headers_of(header_values),
    }
}

pub(super) struct MessagesSession<'fixture> {
    fixture: &'fixture Fixture,
    resolution: ResolutionContext<'fixture>,
    keys: ModelCostKeys,
    dynamic: NoDynamicMetadata,
    runtime: Runtime,
}

impl<'fixture> MessagesSession<'fixture> {
    pub(super) fn new(fixture: &'fixture Fixture) -> Self {
        Self {
            fixture,
            resolution: fixture.resolution(),
            keys: ModelCostKeys::default(),
            dynamic: NoDynamicMetadata,
            runtime: Builder::new_current_thread()
                .build()
                .expect("the test runtime builds"),
        }
    }

    pub(super) fn transform(
        &mut self,
        call: &MessagesCall,
        headers: &mut IndexMap<String, String>,
    ) -> Result<Object, MantleMessagesError> {
        let catalogs = self.fixture.catalogs();
        let mut environment =
            self.fixture
                .capability_environment(&self.resolution, &mut self.keys, &self.dynamic);
        let request = MessagesRequest {
            model: &call.model,
            messages: call.messages.clone(),
            optional_params: call.optional_params.clone(),
            headers: &call.header_values,
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
            .block_on(messages_transform_request(&request, headers, &mut sources))
    }
}

#[derive(Clone, Copy)]
pub(super) enum Mode {
    Sync,
    Async,
}

pub(super) struct ChatSession<'fixture> {
    fixture: &'fixture Fixture,
    resolution: ResolutionContext<'fixture>,
    keys: ModelCostKeys,
    dynamic: NoDynamicMetadata,
    runtime: Runtime,
}

impl<'fixture> ChatSession<'fixture> {
    pub(super) fn new(fixture: &'fixture Fixture) -> Self {
        Self {
            fixture,
            resolution: fixture.resolution(),
            keys: ModelCostKeys::default(),
            dynamic: NoDynamicMetadata,
            runtime: Builder::new_current_thread()
                .build()
                .expect("the test runtime builds"),
        }
    }

    pub(super) fn transform<Fetcher>(
        &mut self,
        request: &ClaudeInvokeRequest<'_>,
        fetcher: &Fetcher,
        mode: Mode,
    ) -> Result<Object, ClaudeInvokeError<Infallible>>
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
        let mut sources = ChatSources {
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
                .block_on(chat_transform_request(request, &mut sources)),
            Mode::Async => self
                .runtime
                .block_on(async_transform_request(request, &mut sources)),
        }
    }
}

pub(super) fn time() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").expect("the fixture is RFC 3339")
}

pub(super) struct FixedClock;

impl SuccessClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        time()
    }
}

pub(super) struct NoDispatch;

#[async_trait]
impl StreamSuccessDispatch for NoDispatch {
    async fn dispatch_success(&self, _call: StreamSuccessCall<'_>) {}
}

pub(super) fn logging_object() -> Arc<StreamLoggingObject> {
    Arc::new(StreamLoggingObject {
        slot: Mutex::new(None),
        dispatch: Arc::new(NoDispatch),
    })
}

#[derive(Debug, Clone)]
pub(super) struct Processed {
    pub(super) response: String,
    pub(super) request_body: IndexMap<String, ParamJson>,
    pub(super) same_logging_object: bool,
    pub(super) url_route: String,
    pub(super) endpoint_anthropic: bool,
}

pub(super) struct Recorder {
    pub(super) processed: Mutex<Option<Processed>>,
    pub(super) supplied: Arc<StreamLoggingObject>,
}

impl StreamSeams for Recorder {
    type Response = String;
    type Stream = String;

    fn route_streaming_logging(
        &self,
        _call: RouteStreamingLogging,
    ) -> Result<LoggingFuture, ReportedFailure> {
        Ok(Box::pin(async { Ok(()) }))
    }

    fn chunk_processor(&self, call: ChunkProcessorCall<String>) -> String {
        let processed = Processed {
            response: call.response,
            request_body: call.request_body,
            same_logging_object: Arc::ptr_eq(&call.logging_obj, &self.supplied),
            url_route: call.url_route.to_owned(),
            endpoint_anthropic: call.endpoint_type == EndpointType::Anthropic,
        };
        let relayed = format!("processed {}", processed.response);
        *self
            .processed
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(processed);
        relayed
    }
}

pub(super) fn recorder_deps(seams: Arc<Recorder>) -> StreamDeps<Recorder> {
    StreamDeps {
        seams,
        worker: Arc::new(
            LoggingWorker::start(WorkerSettings::default()).expect("the worker starts"),
        ),
        slots: Arc::new(StreamSlots::default()),
        clock: Arc::new(FixedClock),
        success_handler: Arc::new(PassThroughEndpointLogging {
            injected_log_dispatch: None,
        }),
    }
}
