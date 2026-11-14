#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::interactions::contract::config::BaseInteractionsAPIConfig;
use betterlite_core::api::interactions::contract::error::InteractionsContractError;
use betterlite_core::api::interactions::contract::records::{
    CompleteUrlRequest, CreateInteractionRequest, EnvironmentRequest, InteractionIdRequest,
    InteractionsLogging, InteractionsUpstream, Params, PreparedRequest, RequestHeaders,
};
use betterlite_core::api::interactions::streaming::async_iterator::InteractionsStreamIterator;
use betterlite_core::api::interactions::streaming::error::InteractionStreamError;
use betterlite_core::api::interactions::streaming::logging::InteractionsStreamLogging;
use betterlite_core::api::interactions::streaming::records::{
    InteractionsApiBase, InteractionsStreamSetup, InteractionsSuccessCall, SharedApiBase,
};
use betterlite_core::api::interactions::streaming::response::{
    InteractionBufferedResponse, InteractionsAsyncResponse, InteractionsLineStream,
    InteractionsSyncResponse,
};
use betterlite_core::api::interactions::streaming::sync_iterator::SyncInteractionsStreamIterator;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_interactions::types::response::{
    CancelInteractionResult, DeleteInteractionResult, InteractionsApiResponse,
    InteractionsStreamChunk,
};
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_models::api_base_lookup::{
    ApiBaseContext, ApiBaseError, ApiBaseHooks, ApiBaseParams, LoginFreeProvider,
};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use chrono::{DateTime, FixedOffset};
use http::{HeaderMap, Method};
use indexmap::IndexMap;
use thiserror::Error;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use url::Url;

const COMPLETED_STATUS: &str = r#"{"id":"int_1","status":"completed"}"#;
const COMPLETED_EVENT: &str = r#"{"id":"int_2","event_type":"interaction.completed"}"#;

#[derive(Debug, Error)]
enum FixtureConfigError {
    #[error("the fixture does not answer this call")]
    Unsupported,
    #[error("the fixture chunk does not decode")]
    Decode(#[source] serde_json::Error),
    #[error("the fixture chunk does not encode")]
    Encode(#[source] serde_json::Error),
    #[error(transparent)]
    Contract(#[from] InteractionsContractError),
}

struct FixtureConfig;

impl BaseInteractionsAPIConfig for FixtureConfig {
    type Error = FixtureConfigError;

    fn custom_llm_provider(&self) -> ProviderId {
        ProviderId::Gemini
    }

    fn get_supported_params(&self, _model: &str) -> Vec<String> {
        Vec::new()
    }

    fn validate_environment(
        &self,
        request: &EnvironmentRequest<'_>,
    ) -> Result<RequestHeaders, Self::Error> {
        Ok(request.headers.clone())
    }

    fn get_complete_url(&self, _request: &CompleteUrlRequest<'_>) -> Result<String, Self::Error> {
        Ok(String::new())
    }

    fn transform_request(
        &self,
        _request: &CreateInteractionRequest<'_>,
    ) -> Result<Params, Self::Error> {
        Ok(Params::new())
    }

    fn transform_response(
        &self,
        _model: Option<&str>,
        _raw_response: &InteractionsUpstream,
        _logging: &mut InteractionsLogging,
    ) -> Result<InteractionsApiResponse, Self::Error> {
        Err(FixtureConfigError::Unsupported)
    }

    fn transform_streaming_response(
        &self,
        _model: Option<&str>,
        parsed_chunk: &JsonValue,
        _logging: &mut InteractionsLogging,
    ) -> Result<InteractionsStreamChunk, Self::Error> {
        let encoded = serde_json::to_string(parsed_chunk).map_err(FixtureConfigError::Encode)?;
        let chunk = serde_json::from_str(&encoded).map_err(FixtureConfigError::Decode)?;
        Ok(chunk)
    }

    fn transform_get_request(
        &self,
        _request: &InteractionIdRequest<'_>,
    ) -> Result<PreparedRequest, Self::Error> {
        Err(FixtureConfigError::Unsupported)
    }

    fn transform_get_response(
        &self,
        _raw_response: &InteractionsUpstream,
        _logging: &mut InteractionsLogging,
    ) -> Result<InteractionsApiResponse, Self::Error> {
        Err(FixtureConfigError::Unsupported)
    }

    fn transform_delete_request(
        &self,
        _request: &InteractionIdRequest<'_>,
    ) -> Result<PreparedRequest, Self::Error> {
        Err(FixtureConfigError::Unsupported)
    }

    fn transform_delete_response(
        &self,
        _raw_response: &InteractionsUpstream,
        _logging: &mut InteractionsLogging,
        _interaction_id: &str,
    ) -> Result<DeleteInteractionResult, Self::Error> {
        Err(FixtureConfigError::Unsupported)
    }

    fn transform_cancel_request(
        &self,
        _request: &InteractionIdRequest<'_>,
    ) -> Result<PreparedRequest, Self::Error> {
        Err(FixtureConfigError::Unsupported)
    }

    fn transform_cancel_response(
        &self,
        _raw_response: &InteractionsUpstream,
        _logging: &mut InteractionsLogging,
    ) -> Result<CancelInteractionResult, Self::Error> {
        Err(FixtureConfigError::Unsupported)
    }
}

struct NoApiBase;

static NO_API_BASE: NoApiBase = NoApiBase;

impl InteractionsApiBase for NoApiBase {
    fn resolve(
        &self,
        _model: &str,
        _params: &ApiBaseParams,
    ) -> Result<Option<String>, ApiBaseError> {
        Ok(None)
    }
}

#[derive(Debug, Error)]
#[error("the fixture stream failed")]
struct FixtureSourceError;

struct FakeAsyncResponse {
    headers: IndexMap<String, String>,
    lines: Vec<Result<String, FixtureSourceError>>,
    calls: Arc<AtomicUsize>,
}

impl FakeAsyncResponse {
    fn new(lines: &[&str]) -> Self {
        Self {
            headers: IndexMap::new(),
            lines: lines.iter().map(|line| Ok(String::from(*line))).collect(),
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

struct FakeAsyncLines {
    lines: std::vec::IntoIter<Result<String, FixtureSourceError>>,
}

#[async_trait]
impl InteractionsAsyncResponse for FakeAsyncResponse {
    type Error = FixtureSourceError;
    type Lines = FakeAsyncLines;

    fn headers(&self) -> IndexMap<String, String> {
        self.headers.clone()
    }

    fn aiter_lines(self) -> Self::Lines {
        self.calls.fetch_add(1, Ordering::SeqCst);
        FakeAsyncLines {
            lines: self.lines.into_iter(),
        }
    }
}

#[async_trait]
impl InteractionsLineStream for FakeAsyncLines {
    type Error = FixtureSourceError;

    async fn next_line(&mut self) -> Result<Option<String>, Self::Error> {
        match self.lines.next() {
            None => Ok(None),
            Some(Ok(line)) => Ok(Some(line)),
            Some(Err(error)) => Err(error),
        }
    }
}

struct FakeSyncResponse {
    headers: IndexMap<String, String>,
    lines: Vec<Result<String, FixtureSourceError>>,
    calls: Arc<AtomicUsize>,
}

impl FakeSyncResponse {
    fn new(lines: &[&str]) -> Self {
        Self {
            headers: IndexMap::new(),
            lines: lines.iter().map(|line| Ok(String::from(*line))).collect(),
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl InteractionsSyncResponse for FakeSyncResponse {
    type Error = FixtureSourceError;
    type Lines = std::vec::IntoIter<Result<String, FixtureSourceError>>;

    fn headers(&self) -> IndexMap<String, String> {
        self.headers.clone()
    }

    fn iter_lines(self) -> Self::Lines {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.lines.into_iter()
    }
}

struct FixedClock {
    now: DateTime<FixedOffset>,
}

impl SuccessClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        self.now
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DispatchRecord {
    handler: &'static str,
    response: InteractionsStreamChunk,
    start_time: DateTime<FixedOffset>,
    end_time: DateTime<FixedOffset>,
    cache_hit: Option<bool>,
    prefer_async_handlers: Option<bool>,
}

struct RecordingLogging {
    logging: InteractionsLogging,
    clock: FixedClock,
    records: Mutex<Vec<DispatchRecord>>,
}

impl RecordingLogging {
    fn new(litellm_params: Params) -> Self {
        Self {
            logging: InteractionsLogging {
                model_call_details: litellm_params,
            },
            clock: FixedClock { now: moment() },
            records: Mutex::new(Vec::new()),
        }
    }

    fn records(&self) -> Vec<DispatchRecord> {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(
        &self,
        handler: &'static str,
        call: InteractionsSuccessCall,
        prefer_async_handlers: Option<bool>,
    ) {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(DispatchRecord {
                handler,
                response: call.response,
                start_time: call.start_time,
                end_time: call.end_time,
                cache_hit: call.cache_hit,
                prefer_async_handlers,
            });
    }
}

impl InteractionsStreamLogging for RecordingLogging {
    fn logging(&mut self) -> &mut InteractionsLogging {
        &mut self.logging
    }

    fn clock(&self) -> &dyn SuccessClock {
        &self.clock
    }

    fn dispatch_success_handlers(&self, call: InteractionsSuccessCall) {
        self.record(
            "dispatch_success_handlers",
            call,
            Some(call.prefer_async_handlers),
        );
    }

    fn run_async_success_handler(&self, call: InteractionsSuccessCall) {
        self.record("async_success_handler", call, None);
    }

    fn submit_success_handler(&self, call: InteractionsSuccessCall) {
        self.record("success_handler", call, None);
    }
}

struct NoHooks;

impl ApiBaseHooks for NoHooks {
    fn login_free_api_base(&self, _provider: LoginFreeProvider) -> Option<String> {
        None
    }

    fn vertex_claude_url(
        &self,
        _location: &str,
        _project: &str,
        _model: &str,
        _stream: bool,
    ) -> Result<String, ApiBaseError> {
        Err(ApiBaseError::InvalidVertexLocation {
            message: String::new(),
        })
    }
}

fn moment() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-05-20T00:00:00+00:00").expect("the fixture time parses")
}

fn litellm_params(api_base: &str) -> Params {
    let fields = IndexMap::from([(
        String::from("api_base"),
        JsonValue::Text(api_base.to_owned()),
    )]);
    Params::from([(String::from("litellm_params"), JsonValue::Object(fields))])
}

fn metadata(model_id: &str) -> Params {
    let info = IndexMap::from([(String::from("id"), JsonValue::Text(model_id.to_owned()))]);
    Params::from([(String::from("model_info"), JsonValue::Object(info))])
}

type TestAsyncIterator<'base> =
    InteractionsStreamIterator<'base, FakeAsyncResponse, FixtureConfig, RecordingLogging>;
type TestSyncIterator<'base> =
    SyncInteractionsStreamIterator<'base, FakeSyncResponse, FixtureConfig, RecordingLogging>;

fn setup<'base>(
    logging: &'base mut RecordingLogging,
    config: &'base FixtureConfig,
    api_base: &'base dyn InteractionsApiBase,
) -> InteractionsStreamSetup<'base, FixtureConfig, RecordingLogging> {
    InteractionsStreamSetup {
        model: Some("gemini-2.5-flash"),
        config,
        logging,
        litellm_metadata: None,
        custom_llm_provider: None,
        api_base,
    }
}

async fn drain_async(iterator: &mut TestAsyncIterator<'_>) -> Vec<InteractionsStreamChunk> {
    let mut events = Vec::new();
    while let Some(item) = iterator.next_chunk().await {
        events.push(item.expect("the fixture stream yields a chunk"));
    }
    events
}

fn drain_sync(iterator: &mut TestSyncIterator<'_>) -> Vec<InteractionsStreamChunk> {
    let mut events = Vec::new();
    while let Some(item) = iterator.next_chunk() {
        events.push(item.expect("the fixture stream yields a chunk"));
    }
    events
}

fn ids(events: &[InteractionsStreamChunk]) -> Vec<&str> {
    events
        .iter()
        .map(|event| event.id.as_deref().unwrap_or_default())
        .collect()
}

fn resolution_context<'context>(
    catalog: &'context ModelCatalog,
    known: &'context BTreeSet<String>,
    custom: &'context BTreeSet<String>,
    environment: &'context BTreeMap<String, String>,
) -> ResolutionContext<'context> {
    ResolutionContext {
        catalog,
        known_model_names: known,
        custom_providers: custom,
        environment,
        registry: JSONProviderRegistry::load(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    }
}

#[tokio::test]
async fn async_iterator_construction_builds_hidden_params_from_the_setup() {
    let catalog = ModelCatalog::default();
    let known = BTreeSet::new();
    let custom = BTreeSet::new();
    let environment = BTreeMap::new();
    let resolution = resolution_context(&catalog, &known, &custom, &environment);
    let aliases = BTreeMap::new();
    let hooks = NoHooks;
    let shared = SharedApiBase {
        context: ApiBaseContext {
            model_alias_map: &aliases,
            hooks: &hooks,
            resolution: &resolution,
        },
    };
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(alias_params("https://alias.test"));
    let mut response = FakeAsyncResponse::new(&[r#"data: {"id":"int_1"}"#]);
    response.headers.insert(
        String::from("x-ratelimit-remaining-requests"),
        String::from("9"),
    );
    let calls = Arc::clone(&response.calls);
    let model_metadata = metadata("model-1");
    let mut iterator = InteractionsStreamIterator::new(
        response,
        InteractionsStreamSetup {
            model: Some("gemini-2.5-flash"),
            config: &config,
            logging: &mut logging,
            litellm_metadata: Some(&model_metadata),
            custom_llm_provider: Some("gemini"),
            api_base: &shared,
        },
    )
    .expect("the fixture iterator builds");
    assert_eq!(
        iterator.base.hidden_params.model_id.as_deref(),
        Some("model-1")
    );
    assert_eq!(
        iterator.base.hidden_params.api_base.as_deref(),
        Some("https://alias.test")
    );
    assert_eq!(
        iterator
            .base
            .hidden_params
            .additional_headers
            .get("x-ratelimit-remaining-requests"),
        Some(&JsonValue::Text(String::from("9")))
    );
    assert_eq!(
        iterator
            .base
            .hidden_params
            .additional_headers
            .get("llm_provider-x-ratelimit-remaining-requests"),
        Some(&JsonValue::Text(String::from("9")))
    );
    assert_eq!(iterator.base.model.as_deref(), Some("gemini-2.5-flash"));
    assert_eq!(iterator.base.custom_llm_provider.as_deref(), Some("gemini"));
    assert!(!iterator.base.finished);
    assert_eq!(iterator.base.completed_response, None);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(drain_async(&mut iterator).await.len(), 1);
}

#[tokio::test]
async fn async_iterator_construction_without_metadata_leaves_model_id_empty() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator = InteractionsStreamIterator::new(
        FakeAsyncResponse::new(&[]),
        setup(&mut logging, &config, &NO_API_BASE),
    )
    .expect("the fixture iterator builds");
    assert_eq!(iterator.base.hidden_params.model_id, None);
    assert_eq!(iterator.base.hidden_params.api_base, None);
    assert!(iterator.next_chunk().await.is_none());
}

#[test]
fn sync_iterator_construction_calls_iter_lines_once() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(litellm_params("https://sync.test"));
    let response = FakeSyncResponse::new(&[r#"data: {"id":"int_1"}"#]);
    let calls = Arc::clone(&response.calls);
    let mut iterator =
        SyncInteractionsStreamIterator::new(response, setup(&mut logging, &config, &NO_API_BASE))
            .expect("the fixture iterator builds");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        iterator.base.hidden_params.api_base.as_deref(),
        Some("https://sync.test")
    );
    assert_eq!(drain_sync(&mut iterator).len(), 1);
}

#[test]
fn sync_iterator_construction_rejects_a_wrongly_typed_api_base() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::from([(
        String::from("litellm_params"),
        JsonValue::Object(IndexMap::from([(
            String::from("api_base"),
            JsonValue::Bool(true),
        )])),
    )]));
    let error = SyncInteractionsStreamIterator::new(
        FakeSyncResponse::new(&[]),
        InteractionsStreamSetup {
            model: None,
            config: &config,
            logging: &mut logging,
            litellm_metadata: None,
            custom_llm_provider: None,
            api_base: &NO_API_BASE,
        },
    )
    .expect_err("a boolean api base is refused");
    assert!(matches!(error, InteractionStreamError::FieldType(_)));
}

#[tokio::test]
async fn async_iterator_construction_calls_aiter_lines_once() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let response = FakeAsyncResponse::new(&[]);
    let calls = Arc::clone(&response.calls);
    let mut iterator =
        InteractionsStreamIterator::new(response, setup(&mut logging, &config, &NO_API_BASE))
            .expect("the fixture iterator builds");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(drain_async(&mut iterator).await.len(), 0);
    assert!(iterator.base.finished);
}

#[tokio::test]
async fn chunk_processing_strips_the_sse_data_prefix() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator = InteractionsStreamIterator::new(
        FakeAsyncResponse::new(&[r#"data: {"id":"a"}"#, r#"data:{"id":"b"}"#, r#"{"id":"c"}"#]),
        setup(&mut logging, &config, &NO_API_BASE),
    )
    .expect("the fixture iterator builds");
    let events = drain_async(&mut iterator).await;
    assert_eq!(ids(&events), vec!["a", "b", "c"]);
}

#[tokio::test]
async fn chunk_processing_ignores_blank_lines() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator = InteractionsStreamIterator::new(
        FakeAsyncResponse::new(&["", r#"data: {"id":"a"}"#, "", "", r#"data: {"id":"b"}"#]),
        setup(&mut logging, &config, &NO_API_BASE),
    )
    .expect("the fixture iterator builds");
    let events = drain_async(&mut iterator).await;
    assert_eq!(ids(&events), vec!["a", "b"]);
}

#[tokio::test]
async fn chunk_processing_ignores_malformed_json() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator = InteractionsStreamIterator::new(
        FakeAsyncResponse::new(&["data: not json", r#"data: {"id":"after"}"#, "data: {"]),
        setup(&mut logging, &config, &NO_API_BASE),
    )
    .expect("the fixture iterator builds");
    let events = drain_async(&mut iterator).await;
    assert_eq!(ids(&events), vec!["after"]);
}

#[tokio::test]
async fn chunk_processing_ignores_non_object_json() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator = InteractionsStreamIterator::new(
        FakeAsyncResponse::new(&[
            "data: [1, 2]",
            "data: \"text\"",
            "data: 7",
            "data: null",
            r#"data: {"id":"ok"}"#,
        ]),
        setup(&mut logging, &config, &NO_API_BASE),
    )
    .expect("the fixture iterator builds");
    let events = drain_async(&mut iterator).await;
    assert_eq!(ids(&events), vec!["ok"]);
}

#[tokio::test]
async fn chunk_processing_treats_done_as_the_end() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator = InteractionsStreamIterator::new(
        FakeAsyncResponse::new(&[
            r#"data: {"id":"first"}"#,
            "data: [DONE]",
            r#"data: {"id":"after"}"#,
        ]),
        setup(&mut logging, &config, &NO_API_BASE),
    )
    .expect("the fixture iterator builds");
    let events = drain_async(&mut iterator).await;
    assert_eq!(ids(&events), vec!["first"]);
    assert!(iterator.base.finished);
}

#[tokio::test]
async fn chunk_processing_detects_the_legacy_completed_status() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator = InteractionsStreamIterator::new(
        FakeAsyncResponse::new(&[&format!("data: {COMPLETED_STATUS}")]),
        setup(&mut logging, &config, &NO_API_BASE),
    )
    .expect("the fixture iterator builds");
    let events = drain_async(&mut iterator).await;
    assert_eq!(ids(&events), vec!["int_1"]);
    assert_eq!(
        iterator
            .base
            .completed_response
            .as_ref()
            .and_then(|response| response.id.as_deref()),
        Some("int_1")
    );
    assert_eq!(logging.records().len(), 1);
}

#[tokio::test]
async fn chunk_processing_detects_the_new_completed_event_type() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator = InteractionsStreamIterator::new(
        FakeAsyncResponse::new(&[&format!("data: {COMPLETED_EVENT}")]),
        setup(&mut logging, &config, &NO_API_BASE),
    )
    .expect("the fixture iterator builds");
    let events = drain_async(&mut iterator).await;
    assert_eq!(ids(&events), vec!["int_2"]);
    assert_eq!(logging.records().len(), 1);
}

#[tokio::test]
async fn async_iteration_yields_events_in_order_and_finishes() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator = InteractionsStreamIterator::new(
        FakeAsyncResponse::new(&[
            r#"data: {"id":"a","event_type":"interaction.created"}"#,
            r#"data: {"id":"b","event_type":"step.delta","index":0}"#,
        ]),
        setup(&mut logging, &config, &NO_API_BASE),
    )
    .expect("the fixture iterator builds");
    let events = drain_async(&mut iterator).await;
    assert_eq!(ids(&events), vec!["a", "b"]);
    assert!(iterator.base.finished);
    assert!(logging.records().is_empty());
    assert!(iterator.next_chunk().await.is_none());
}

#[tokio::test]
async fn async_iteration_dispatches_completion_with_prefer_async() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator = InteractionsStreamIterator::new(
        FakeAsyncResponse::new(&[&format!("data: {COMPLETED_STATUS}")]),
        setup(&mut logging, &config, &NO_API_BASE),
    )
    .expect("the fixture iterator builds");
    drain_async(&mut iterator).await;
    let records = logging.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].handler, "dispatch_success_handlers");
    assert_eq!(records[0].response.id.as_deref(), Some("int_1"));
    assert_eq!(records[0].start_time, moment());
    assert_eq!(records[0].end_time, moment());
    assert_eq!(records[0].cache_hit, None);
    assert_eq!(records[0].prefer_async_handlers, Some(true));
}

#[tokio::test]
async fn async_iteration_skips_empty_results_between_events() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator = InteractionsStreamIterator::new(
        FakeAsyncResponse::new(&["data: ", "", r#"data: {"id":"only"}"#, "data: [DONE]"]),
        setup(&mut logging, &config, &NO_API_BASE),
    )
    .expect("the fixture iterator builds");
    let events = drain_async(&mut iterator).await;
    assert_eq!(ids(&events), vec!["only"]);
}

#[test]
fn sync_iteration_yields_events_in_order_and_finishes() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator = SyncInteractionsStreamIterator::new(
        FakeSyncResponse::new(&[r#"data: {"id":"a"}"#, r#"data: {"id":"b"}"#, "data: [DONE]"]),
        setup(&mut logging, &config, &NO_API_BASE),
    )
    .expect("the fixture iterator builds");
    let events = drain_sync(&mut iterator);
    assert_eq!(ids(&events), vec!["a", "b"]);
    assert!(iterator.base.finished);
    assert!(iterator.next_chunk().is_none());
}

#[test]
fn sync_iteration_dispatches_the_async_handler_then_the_sync_handler() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator = SyncInteractionsStreamIterator::new(
        FakeSyncResponse::new(&[&format!("data: {COMPLETED_EVENT}")]),
        setup(&mut logging, &config, &NO_API_BASE),
    )
    .expect("the fixture iterator builds");
    drain_sync(&mut iterator);
    let records = logging.records();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].handler, "async_success_handler");
    assert_eq!(records[1].handler, "success_handler");
    assert_eq!(records[0].response, records[1].response);
    assert_eq!(records[0].response.id.as_deref(), Some("int_2"));
    assert_eq!(records[0].start_time, moment());
    assert_eq!(records[1].start_time, moment());
    assert_eq!(records[0].end_time, moment());
    assert_eq!(records[1].end_time, moment());
    assert_eq!(records[0].cache_hit, None);
    assert_eq!(records[1].cache_hit, None);
    assert_eq!(records[0].prefer_async_handlers, None);
    assert_eq!(records[1].prefer_async_handlers, None);
}

async fn serve_stream(listener: TcpListener, answer: &str) {
    let (mut stream, _) = listener.accept().await.expect("the client connects");
    let mut head = Vec::new();
    let mut byte = [0_u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        stream
            .read_exact(&mut byte)
            .await
            .expect("the request head arrives");
        head.push(byte[0]);
    }
    let reply = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nx-ratelimit-remaining-requests: 9\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{answer}",
        answer.len()
    );
    stream
        .write_all(reply.as_bytes())
        .await
        .expect("the reply is written");
    stream.shutdown().await.expect("the connection closes");
}

#[tokio::test]
async fn async_iteration_reads_a_streamed_http_response() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the port binds");
    let address = listener.local_addr().expect("the address resolves");
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
        .expect("the proxy policy resolves");
    let client = ProxiedClient::outbound(&Arc::new(policy)).expect("the client builds");
    let request = OutboundRequest {
        method: Method::GET,
        url: Url::parse(&format!("http://{address}/stream")).expect("the url parses"),
        query: Vec::new(),
        headers: HeaderMap::new(),
        body: RequestBody::Empty,
        deadline: Duration::from_secs(30),
    };
    let answer = format!("data: {COMPLETED_STATUS}\n\ndata: [DONE]\n");
    let (response, ()) = tokio::join!(client.send(request), serve_stream(listener, &answer));
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator = InteractionsStreamIterator::new(
        response.expect("the stream response arrives"),
        setup(&mut logging, &config, &NO_API_BASE),
    )
    .expect("the http iterator builds");
    assert_eq!(
        iterator
            .base
            .hidden_params
            .additional_headers
            .get("x-ratelimit-remaining-requests"),
        Some(&JsonValue::Text(String::from("9")))
    );
    let first = iterator
        .next_chunk()
        .await
        .expect("the first event arrives")
        .expect("the first event decodes");
    assert_eq!(first.id.as_deref(), Some("int_1"));
    assert!(iterator.next_chunk().await.is_none());
    assert!(iterator.base.finished);
}

#[test]
fn sync_iteration_reads_a_buffered_response() {
    let response = InteractionBufferedResponse {
        headers: IndexMap::from([(
            String::from("x-ratelimit-remaining-requests"),
            String::from("9"),
        )]),
        body: format!("data: {COMPLETED_EVENT}\n\ndata: [DONE]\n").into_bytes(),
    };
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator =
        SyncInteractionsStreamIterator::new(response, setup(&mut logging, &config, &NO_API_BASE))
            .expect("the buffered iterator builds");
    assert_eq!(
        iterator
            .base
            .hidden_params
            .additional_headers
            .get("x-ratelimit-remaining-requests"),
        Some(&JsonValue::Text(String::from("9")))
    );
    let first = iterator
        .next_chunk()
        .expect("the first event arrives")
        .expect("the first event decodes");
    assert_eq!(first.id.as_deref(), Some("int_2"));
    assert!(iterator.next_chunk().is_none());
    assert_eq!(logging.records().len(), 2);
}

#[tokio::test]
async fn error_handling_async_source_error_sets_finished_and_errors() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut response = FakeAsyncResponse::new(&[r#"data: {"id":"a"}"#]);
    response.lines.push(Err(FixtureSourceError));
    let mut iterator =
        InteractionsStreamIterator::new(response, setup(&mut logging, &config, &NO_API_BASE))
            .expect("the fixture iterator builds");
    assert!(
        iterator
            .next_chunk()
            .await
            .expect("the first chunk")
            .is_ok()
    );
    let failure = iterator
        .next_chunk()
        .await
        .expect("the stream reports an error");
    assert!(matches!(failure, Err(InteractionStreamError::Source(_))));
    assert!(iterator.base.finished);
    assert!(iterator.next_chunk().await.is_none());
}

#[test]
fn error_handling_sync_source_error_sets_finished_and_errors() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut response = FakeSyncResponse::new(&[]);
    response.lines.push(Err(FixtureSourceError));
    let mut iterator =
        SyncInteractionsStreamIterator::new(response, setup(&mut logging, &config, &NO_API_BASE))
            .expect("the fixture iterator builds");
    let failure = iterator.next_chunk().expect("the stream reports an error");
    assert!(matches!(failure, Err(InteractionStreamError::Source(_))));
    assert!(iterator.base.finished);
    assert!(iterator.next_chunk().is_none());
}

#[tokio::test]
async fn error_handling_transform_error_is_returned_without_finishing() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator = InteractionsStreamIterator::new(
        FakeAsyncResponse::new(&[r#"data: {"id":5}"#, r#"data: {"id":"int_1"}"#]),
        setup(&mut logging, &config, &NO_API_BASE),
    )
    .expect("the fixture iterator builds");
    let failure = iterator
        .next_chunk()
        .await
        .expect("the chunk is transformed");
    assert!(matches!(failure, Err(InteractionStreamError::Transform(_))));
    assert!(!iterator.base.finished);
    let next = iterator.next_chunk().await.expect("the stream continues");
    assert_eq!(
        next.expect("the second chunk decodes").id.as_deref(),
        Some("int_1")
    );
}

#[tokio::test]
async fn multiple_completions_keep_the_last_response() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator = InteractionsStreamIterator::new(
        FakeAsyncResponse::new(&[
            &format!("data: {COMPLETED_STATUS}"),
            r#"data: {"id":"int_3","event_type":"step.delta"}"#,
            &format!("data: {COMPLETED_EVENT}"),
        ]),
        setup(&mut logging, &config, &NO_API_BASE),
    )
    .expect("the fixture iterator builds");
    let events = drain_async(&mut iterator).await;
    assert_eq!(ids(&events), vec!["int_1", "int_3", "int_2"]);
    assert_eq!(
        iterator
            .base
            .completed_response
            .as_ref()
            .and_then(|response| response.id.as_deref()),
        Some("int_2")
    );
    let records = logging.records();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].response.id.as_deref(), Some("int_1"));
    assert_eq!(records[1].response.id.as_deref(), Some("int_2"));
}

fn alias_params(base_url: &str) -> Params {
    let fields = IndexMap::from([(
        String::from("base_url"),
        JsonValue::Text(base_url.to_owned()),
    )]);
    Params::from([(String::from("litellm_params"), JsonValue::Object(fields))])
}
