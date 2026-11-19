#![cfg(test)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_error::error::ErrorClass;
use betterlite_error::error::details::{MidStreamFallbackDetails, TimeoutDetails};
use betterlite_http::error::HttpError;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_logging::logging_success::types::SuccessRequest;
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamLoggingState, StreamSuccessCall, StreamSuccessDispatch,
};
use betterlite_models::api_base_lookup::{ApiBaseError, ApiBaseParams};
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::ResponsesAPIStreamingResponse;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use thiserror::Error;

use super::errors::{DriverError, TransportFailure, TransportKind};
use super::live::{Event, LiveStream, StreamEngine};
use super::source::{PayloadSource, PrefetchedSource, SourceFailure, classify};
use super::synthetic::{SyntheticSetup, SyntheticStream, is_completed};
use crate::api::api_boundaries::ApiLimits;
use crate::api::response_synthetic_events::estimate::{
    EstimateRequest, TokenCountRequest, UsageTokenization,
};
use crate::api::response_synthetic_events::stamping::{CostTarget, ResponsesCostCalculator};
use crate::api::responses_stream_events::{
    BaseStreamStateInput, LoggingSnapshot, ResponsesApiBase, ResponsesEventTransform, StreamTimes,
};
use crate::api::responses_stream_hooks::ResponsesStreamHooks;
use crate::api::responses_stream_hooks::ports::{
    CacheWrite, ChunkDeploymentHook, ChunkHookCall, DynamicCacheObject, FailureDelivery,
    PortFailure, ResponsesStreamCacheHandler, ResponsesStreamCacheTarget,
    ResponsesStreamFailureTarget, ResponsesStreamSuccessTarget,
};

const START: &str = "2023-11-14T22:13:20+00:00";
const CLOCK: &str = "2023-11-14T22:13:21+00:00";
const LATE: &str = "2023-11-14T22:13:40+00:00";
const CREATED_AT: f64 = 1_700_000_000.0;

const CREATED: &str = r#"{"type":"response.created","sequence_number":0,"response":{"id":"resp_1","created_at":1700000000,"object":"response","status":"in_progress","output":[]}}"#;
const IN_PROGRESS: &str = r#"{"type":"response.in_progress","sequence_number":1,"response":{"id":"resp_1","created_at":1700000000,"object":"response","status":"in_progress","output":[]}}"#;
const QUEUED: &str = r#"{"type":"response.queued","sequence_number":0,"response":{"id":"resp_1","created_at":1700000000,"object":"response","status":"queued","output":[]}}"#;
const DELTA: &str = r#"{"type":"response.output_text.delta","sequence_number":2,"item_id":"msg_1","output_index":0,"content_index":0,"delta":"hi"}"#;
const COMPLETED: &str = r#"{"type":"response.completed","sequence_number":3,"response":{"id":"resp_1","created_at":1700000000,"object":"response","status":"completed","output":[]}}"#;
const RATE_LIMITED: &str = r#"{"type":"error","sequence_number":4,"error":{"type":"rate_limit_exceeded","code":"rate_limit_exceeded","message":"slow down"}}"#;
const SERVER_ERROR: &str = r#"{"type":"error","sequence_number":4,"error":{"type":"server_error","code":"server_error","message":"boom"}}"#;
const BAD_REQUEST: &str = r#"{"type":"error","sequence_number":4,"error":{"type":"invalid_request_error","code":"model_not_found","message":"no such model"}}"#;
const DONE: &str = "[DONE]";

const RESPONSE_FIXTURE: &str = r#"{"id":"resp_a","created_at":1700000000,"status":"completed","output":[{"type":"message","id":"msg_1","status":"completed","role":"assistant","content":[{"type":"output_text","text":"abcdefghijk","annotations":[],"logprobs":null}]}]}"#;

fn timestamp(text: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(text).expect("the fixture time parses")
}

fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds")
}

fn payload(text: &str) -> Result<Vec<u8>, SourceFailure> {
    Ok(text.as_bytes().to_vec())
}

fn transport(kind: TransportKind, message: &str) -> Result<Vec<u8>, SourceFailure> {
    Err(SourceFailure::Transport(TransportFailure {
        kind,
        message: message.to_owned(),
    }))
}

fn field(event: &Event, key: &str) -> String {
    serde_json::to_value(event)
        .expect("the event serializes")
        .get(key)
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_owned()
}

fn event_type(event: &Event) -> String {
    field(event, "type")
}

fn delta(event: &Event) -> String {
    field(event, "delta")
}

fn fallback(error: &DriverError) -> &MidStreamFallbackDetails {
    match error {
        DriverError::Class {
            class: ErrorClass::MidStreamFallback(details),
        } => details,
        other => panic!("expected a mid-stream fallback, got {other:?}"),
    }
}

fn connection_message(error: &DriverError) -> &str {
    match error {
        DriverError::Class {
            class: ErrorClass::ApiConnection(context),
        } => &context.message,
        other => panic!("expected an api connection error, got {other:?}"),
    }
}

fn timeout_details(error: &DriverError) -> &TimeoutDetails {
    match error {
        DriverError::Class {
            class: ErrorClass::Timeout(details),
        } => details,
        other => panic!("expected a streaming timeout, got {other:?}"),
    }
}

fn class_of(error: &DriverError) -> &ErrorClass {
    match error {
        DriverError::Class { class } => class,
        DriverError::Transport(failure) => panic!("expected a class, got {failure:?}"),
    }
}

struct NoApiBase;

impl ResponsesApiBase for NoApiBase {
    fn resolve(
        &self,
        _model: &str,
        _params: &ApiBaseParams,
    ) -> Result<Option<String>, ApiBaseError> {
        Ok(None)
    }
}

#[derive(Debug, Error)]
enum TransformError {
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

struct DecodingTransform;

impl ResponsesEventTransform for DecodingTransform {
    type Error = TransformError;

    fn transform(
        &self,
        _model: &str,
        chunk: &Json,
        _logging: &mut IndexMap<String, Json>,
    ) -> Result<ResponsesAPIStreamingResponse, Self::Error> {
        let value = serde_json::to_value(chunk)?;
        Ok(serde_json::from_value(value)?)
    }
}

struct NoTokenization;

impl UsageTokenization for NoTokenization {
    type Error = std::io::Error;

    fn messages(&self, _request: &EstimateRequest<'_>) -> Result<Vec<Json>, Self::Error> {
        Ok(Vec::new())
    }

    fn count(&self, _request: TokenCountRequest<'_>) -> Result<i64, Self::Error> {
        Ok(0)
    }
}

struct NoCost;

impl ResponsesCostCalculator for NoCost {
    type Error = std::io::Error;

    fn response_cost(&self, _target: &CostTarget<'_>) -> Result<Option<Decimal>, Self::Error> {
        Ok(None)
    }
}

struct FixedClock(DateTime<FixedOffset>);

impl SuccessClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        self.0
    }
}

struct ScriptedClock {
    times: Mutex<VecDeque<DateTime<FixedOffset>>>,
    fallback: DateTime<FixedOffset>,
}

impl SuccessClock for ScriptedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        let mut times = self.times.lock().unwrap_or_else(PoisonError::into_inner);
        times.pop_front().unwrap_or(self.fallback)
    }
}

fn scripted_clock(times: &[&str]) -> Arc<dyn SuccessClock> {
    Arc::new(ScriptedClock {
        times: Mutex::new(times.iter().map(|text| timestamp(text)).collect()),
        fallback: timestamp(LATE),
    })
}

fn fixed_clock() -> Arc<dyn SuccessClock> {
    Arc::new(FixedClock(timestamp(CLOCK)))
}

struct NoDispatch;

#[async_trait]
impl StreamSuccessDispatch for NoDispatch {
    async fn dispatch_success(&self, _call: StreamSuccessCall<'_>) {}
}

fn logging_object() -> StreamLoggingObject {
    let mut context = CorrelationContext::default();
    let logging = LoggingState::new(
        LoggingInit {
            model: "gpt-4o".to_owned(),
            call_type: "responses".to_owned(),
            start_time: timestamp(START),
            litellm_call_id: "call-1".to_owned(),
            function_id: "fn-1".to_owned(),
            messages: None,
            stream: Some(true),
            litellm_trace_id: Some("trace-1".to_owned()),
            kwargs: IndexMap::new(),
            litellm_params: IndexMap::new(),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        },
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &redactor(),
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .expect("the logging fixture builds");
    StreamLoggingObject {
        slot: Mutex::new(Some(StreamLoggingState {
            logging,
            correlation: context,
        })),
        dispatch: Arc::new(NoDispatch),
    }
}

#[derive(Default)]
struct SuccessPort {
    dispatches: Mutex<Vec<SuccessRequest>>,
    async_calls: Mutex<Vec<SuccessRequest>>,
    sync_calls: Mutex<Vec<SuccessRequest>>,
}

impl SuccessPort {
    fn dispatches(&self) -> Vec<SuccessRequest> {
        self.dispatches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn async_calls(&self) -> Vec<SuccessRequest> {
        self.async_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn sync_calls(&self) -> Vec<SuccessRequest> {
        self.sync_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl ResponsesStreamSuccessTarget for SuccessPort {
    async fn dispatch_success_handlers(&self, request: SuccessRequest) {
        self.dispatches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
    }

    async fn async_success_handler(&self, request: &SuccessRequest) {
        self.async_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
    }

    fn success_handler(&self, request: &SuccessRequest) {
        self.sync_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
    }

    fn deferred_armed(&self) -> bool {
        false
    }
}

#[derive(Default)]
struct FailurePort {
    classes: Mutex<Vec<ErrorClass>>,
}

impl FailurePort {
    fn classes(&self) -> Vec<ErrorClass> {
        self.classes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl ResponsesStreamFailureTarget for FailurePort {
    async fn async_failure_handler(&self, delivery: &FailureDelivery) -> Result<(), PortFailure> {
        self.classes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(delivery.exception.clone());
        Ok(())
    }

    fn failure_handler(&self, _delivery: &FailureDelivery) -> Result<(), PortFailure> {
        Ok(())
    }
}

struct ReplacingHook {
    replacement: Event,
    calls: Mutex<Vec<String>>,
}

impl ReplacingHook {
    fn new(replacement: Event) -> Arc<Self> {
        Arc::new(Self {
            replacement,
            calls: Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl ChunkDeploymentHook for ReplacingHook {
    async fn post_streaming_deployment_hook(
        &self,
        call: ChunkHookCall<'_>,
    ) -> Result<Option<Event>, PortFailure> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event_type(call.chunk));
        Ok(Some(self.replacement.clone()))
    }
}

struct CacheHandler {
    kwargs: IndexMap<String, Json>,
}

impl ResponsesStreamCacheHandler for CacheHandler {
    fn request_kwargs(&self) -> Option<&IndexMap<String, Json>> {
        Some(&self.kwargs)
    }

    fn preset_cache_key(&self) -> Option<&str> {
        None
    }

    fn original_function(&self) -> Option<&str> {
        Some("aresponses")
    }

    fn dual_cache(&self) -> Option<DynamicCacheObject> {
        None
    }

    fn should_store_result_in_cache(
        &self,
        _original_function: Option<&str>,
        _kwargs: &IndexMap<String, Json>,
    ) -> Result<bool, PortFailure> {
        Ok(true)
    }
}

fn caching_handler() -> CacheHandler {
    let mut kwargs = IndexMap::new();
    kwargs.insert("stream".to_owned(), Json::Bool(true));
    CacheHandler { kwargs }
}

#[derive(Default)]
struct RecordingCache {
    writes: Mutex<Vec<String>>,
}

impl RecordingCache {
    fn writes(&self) -> Vec<String> {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, write: CacheWrite, asynchronous: bool) {
        let suffix = if asynchronous { ":async" } else { ":sync" };
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("{}{suffix}", write.payload));
    }
}

#[async_trait]
impl ResponsesStreamCacheTarget for RecordingCache {
    fn add_cache(&self, write: CacheWrite) -> Result<(), PortFailure> {
        self.record(write, false);
        Ok(())
    }

    async fn async_add_cache(&self, write: CacheWrite) -> Result<(), PortFailure> {
        self.record(write, true);
        Ok(())
    }
}

#[derive(Default)]
struct Ports {
    success: Option<Arc<dyn ResponsesStreamSuccessTarget>>,
    failure: Option<Arc<dyn ResponsesStreamFailureTarget>>,
    chunk_hooks: Vec<Arc<dyn ChunkDeploymentHook>>,
    cache: Option<Arc<dyn ResponsesStreamCacheTarget>>,
    cache_handler: Option<Arc<dyn ResponsesStreamCacheHandler>>,
}

impl Ports {
    fn hooks<'run>(
        &self,
        redactor: &'run Redactor,
        clock: Arc<dyn SuccessClock>,
    ) -> ResponsesStreamHooks<'run> {
        ResponsesStreamHooks {
            cache: self.cache.clone(),
            cache_handler: self.cache_handler.clone(),
            success: self.success.clone(),
            failure: self.failure.clone(),
            metadata: None,
            success_hook: None,
            chunk_hooks: self.chunk_hooks.clone(),
            clock,
            redactor,
        }
    }
}

struct Fixture {
    redactor: Redactor,
    ports: Ports,
    clock: Arc<dyn SuccessClock>,
    logging: StreamLoggingObject,
    limits: ApiLimits,
    transform: DecodingTransform,
    tokenization: NoTokenization,
    calculator: NoCost,
    api_base: NoApiBase,
    headers: IndexMap<String, String>,
}

impl Fixture {
    fn new(ports: Ports, clock: Arc<dyn SuccessClock>) -> Self {
        Self {
            redactor: redactor(),
            ports,
            clock,
            logging: logging_object(),
            limits: ApiLimits::default(),
            transform: DecodingTransform,
            tokenization: NoTokenization,
            calculator: NoCost,
            api_base: NoApiBase,
            headers: IndexMap::new(),
        }
    }

    fn hooks(&self) -> ResponsesStreamHooks<'_> {
        self.ports.hooks(&self.redactor, Arc::clone(&self.clock))
    }

    fn stream<'run>(
        &'run self,
        hooks: &'run ResponsesStreamHooks<'run>,
        source: Box<dyn PayloadSource>,
        max_streaming_duration_seconds: Option<f64>,
    ) -> LiveStream<'run, DecodingTransform, NoTokenization, NoCost> {
        LiveStream::new(
            StreamEngine {
                transform: Some(&self.transform),
                tokenization: Some(&self.tokenization),
                calculator: Some(&self.calculator),
                hooks,
                logging: &self.logging,
                details: IndexMap::new(),
                limits: &self.limits,
            },
            &BaseStreamStateInput {
                model: "gpt-4o",
                custom_llm_provider: Some("openai"),
                litellm_metadata: None,
                request_data: None,
                call_type: Some("responses"),
                logging: LoggingSnapshot::read(&self.logging),
                headers: &self.headers,
                api_base: &self.api_base,
                times: StreamTimes {
                    now: timestamp(START),
                    created_timestamp: CREATED_AT,
                },
                max_streaming_duration_seconds,
            },
            source,
        )
    }

    fn synthetic<'run>(
        &'run self,
        hooks: &'run ResponsesStreamHooks<'run>,
        cache_hit: Option<bool>,
        persist_before_logging: bool,
    ) -> SyntheticStream<'run> {
        let response: ResponsesApiResponse =
            serde_json::from_str(RESPONSE_FIXTURE).expect("the response fixture decodes");
        SyntheticStream::build(
            response,
            &SyntheticSetup {
                state: BaseStreamStateInput {
                    model: "gpt-4o",
                    custom_llm_provider: Some("openai"),
                    litellm_metadata: None,
                    request_data: None,
                    call_type: Some("responses"),
                    logging: LoggingSnapshot::read(&self.logging),
                    headers: &self.headers,
                    api_base: &self.api_base,
                    times: StreamTimes {
                        now: timestamp(START),
                        created_timestamp: CREATED_AT,
                    },
                    max_streaming_duration_seconds: None,
                },
                hooks,
                cache_hit,
                persist_before_logging,
            },
            Some(&self.calculator),
        )
        .expect("the synthetic events build")
    }
}

struct CountingSource {
    payloads: VecDeque<Vec<u8>>,
    polls: Arc<Mutex<usize>>,
}

impl CountingSource {
    fn new(payloads: &[&str]) -> (Self, Arc<Mutex<usize>>) {
        let polls = Arc::new(Mutex::new(0));
        let source = Self {
            payloads: payloads
                .iter()
                .map(|text| text.as_bytes().to_vec())
                .collect(),
            polls: Arc::clone(&polls),
        };
        (source, polls)
    }
}

#[async_trait]
impl PayloadSource for CountingSource {
    async fn next_payload(&mut self) -> Option<Result<Vec<u8>, SourceFailure>> {
        let mut polls = self.polls.lock().unwrap_or_else(PoisonError::into_inner);
        *polls = polls.saturating_add(1);
        drop(polls);
        self.payloads.pop_front().map(Ok)
    }
}

fn poll_count(polls: &Arc<Mutex<usize>>) -> usize {
    *polls.lock().unwrap_or_else(PoisonError::into_inner)
}

fn prefetched(items: Vec<Result<Vec<u8>, SourceFailure>>) -> Box<dyn PayloadSource> {
    Box::new(PrefetchedSource::new(items))
}

async fn next_typed(
    stream: &mut LiveStream<'_, DecodingTransform, NoTokenization, NoCost>,
) -> Event {
    stream
        .next_event()
        .await
        .expect("the event yields")
        .expect("an event is returned")
}

fn assert_single_success(success: &SuccessPort, failure: &FailurePort) {
    let dispatches = success.dispatches();
    assert_eq!(dispatches.len(), 1);
    assert_eq!(dispatches[0].cache_hit, None);
    assert!(dispatches[0].prefer_async_handlers);
    assert!(success.async_calls().is_empty());
    assert!(failure.classes().is_empty());
}

#[tokio::test]
async fn live_sse_delivery_yields_events_and_finishes() {
    let success = Arc::new(SuccessPort::default());
    let failure = Arc::new(FailurePort::default());
    let mut ports = Ports::default();
    ports.success = Some(Arc::<SuccessPort>::clone(&success));
    ports.failure = Some(Arc::<FailurePort>::clone(&failure));
    let fixture = Fixture::new(ports, fixed_clock());
    let hooks = fixture.hooks();
    let mut stream = fixture.stream(
        &hooks,
        prefetched(vec![
            payload(CREATED),
            payload(DELTA),
            payload(COMPLETED),
            payload(DONE),
        ]),
        None,
    );
    assert_eq!(
        event_type(&next_typed(&mut stream).await),
        "response.created"
    );
    assert_eq!(delta(&next_typed(&mut stream).await), "hi");
    assert_eq!(
        event_type(&next_typed(&mut stream).await),
        "response.completed"
    );
    assert!(
        stream
            .next_event()
            .await
            .expect("the stream ends")
            .is_none()
    );
    assert_single_success(&success, &failure);
}

#[tokio::test]
async fn early_stream_end_before_any_event_raises_a_fallback() {
    let failure = Arc::new(FailurePort::default());
    let mut ports = Ports::default();
    ports.failure = Some(Arc::<FailurePort>::clone(&failure));
    let fixture = Fixture::new(ports, fixed_clock());
    let hooks = fixture.hooks();
    let mut stream = fixture.stream(&hooks, prefetched(Vec::new()), None);
    let error = stream
        .next_event()
        .await
        .expect_err("an empty stream fails");
    let details = fallback(&error);
    assert_eq!(details.original_status.0, 500);
    assert!(details.is_pre_first_chunk);
    assert_eq!(details.generated_content, "");
    assert_eq!(
        details.context.message,
        "litellm.MidStreamFallbackError: litellm.APIConnectionError: openai closed the responses stream before any terminal event (response.completed, response.incomplete or response.failed)"
    );
    assert_eq!(failure.classes().len(), 1);
}

#[tokio::test]
async fn early_stream_end_after_a_lifecycle_event_still_falls_back() {
    let fixture = Fixture::new(Ports::default(), fixed_clock());
    let hooks = fixture.hooks();
    let mut stream = fixture.stream(&hooks, prefetched(vec![payload(CREATED)]), None);
    stream
        .next_event()
        .await
        .expect("the created event yields")
        .expect("an event is returned");
    let error = stream.next_event().await.expect_err("the stream fails");
    let details = fallback(&error);
    assert_eq!(details.original_status.0, 500);
    assert!(!details.is_pre_first_chunk);
}

#[tokio::test]
async fn early_stream_end_after_output_raises_the_connection_error() {
    let failure = Arc::new(FailurePort::default());
    let mut ports = Ports::default();
    ports.failure = Some(Arc::<FailurePort>::clone(&failure));
    let fixture = Fixture::new(ports, fixed_clock());
    let hooks = fixture.hooks();
    let mut stream = fixture.stream(
        &hooks,
        prefetched(vec![payload(CREATED), payload(DELTA)]),
        None,
    );
    stream.next_event().await.expect("the created event yields");
    stream.next_event().await.expect("the delta yields");
    let error = stream.next_event().await.expect_err("the stream fails");
    assert_eq!(
        connection_message(&error),
        "litellm.APIConnectionError: openai closed the responses stream before any terminal event (response.completed, response.incomplete or response.failed)"
    );
    assert_eq!(class_of(&error).metadata().status_code, 500);
    assert_eq!(failure.classes().len(), 1);
}

#[tokio::test]
async fn the_done_marker_without_a_terminal_event_raises_the_early_end_error() {
    let fixture = Fixture::new(Ports::default(), fixed_clock());
    let hooks = fixture.hooks();
    let mut stream = fixture.stream(
        &hooks,
        prefetched(vec![payload(CREATED), payload(DONE)]),
        None,
    );
    stream.next_event().await.expect("the created event yields");
    let error = stream
        .next_event()
        .await
        .expect_err("the done marker fails");
    assert_eq!(fallback(&error).original_status.0, 500);
}

#[tokio::test]
async fn the_done_marker_after_a_terminal_event_ends_the_iteration() {
    let failure = Arc::new(FailurePort::default());
    let mut ports = Ports::default();
    ports.failure = Some(Arc::<FailurePort>::clone(&failure));
    let fixture = Fixture::new(ports, fixed_clock());
    let hooks = fixture.hooks();
    let mut stream = fixture.stream(
        &hooks,
        prefetched(vec![payload(COMPLETED), payload(DONE)]),
        None,
    );
    stream
        .next_event()
        .await
        .expect("the terminal event yields");
    assert!(
        stream
            .next_event()
            .await
            .expect("the stream ends")
            .is_none()
    );
    assert!(failure.classes().is_empty());
}

#[tokio::test]
async fn transport_error_after_a_terminal_event_ends_the_iteration() {
    let failure = Arc::new(FailurePort::default());
    let mut ports = Ports::default();
    ports.failure = Some(Arc::<FailurePort>::clone(&failure));
    let fixture = Fixture::new(ports, fixed_clock());
    let hooks = fixture.hooks();
    let mut stream = fixture.stream(
        &hooks,
        prefetched(vec![
            payload(COMPLETED),
            transport(TransportKind::ReadError, "peer closed the connection"),
        ]),
        None,
    );
    stream
        .next_event()
        .await
        .expect("the terminal event yields");
    assert!(
        stream
            .next_event()
            .await
            .expect("the stream ends")
            .is_none()
    );
    assert!(failure.classes().is_empty());
}

#[tokio::test]
async fn transport_error_before_output_raises_a_fallback_with_503() {
    let failure = Arc::new(FailurePort::default());
    let mut ports = Ports::default();
    ports.failure = Some(Arc::<FailurePort>::clone(&failure));
    let fixture = Fixture::new(ports, fixed_clock());
    let hooks = fixture.hooks();
    let mut stream = fixture.stream(
        &hooks,
        prefetched(vec![transport(
            TransportKind::RemoteProtocolError,
            "peer closed the connection",
        )]),
        None,
    );
    let error = stream.next_event().await.expect_err("the read fails");
    let details = fallback(&error);
    assert_eq!(details.original_status.0, 503);
    assert!(details.is_pre_first_chunk);
    assert_eq!(
        details.context.message,
        "litellm.MidStreamFallbackError: peer closed the connection"
    );
    assert_eq!(failure.classes().len(), 1);
}

#[tokio::test]
async fn transport_error_after_output_raises_the_transport_error() {
    let failure = Arc::new(FailurePort::default());
    let mut ports = Ports::default();
    ports.failure = Some(Arc::<FailurePort>::clone(&failure));
    let fixture = Fixture::new(ports, fixed_clock());
    let hooks = fixture.hooks();
    let mut stream = fixture.stream(
        &hooks,
        prefetched(vec![
            payload(CREATED),
            payload(DELTA),
            transport(TransportKind::ReadError, "peer closed the connection"),
        ]),
        None,
    );
    stream.next_event().await.expect("the created event yields");
    stream.next_event().await.expect("the delta yields");
    let error = stream.next_event().await.expect_err("the read fails");
    match error {
        DriverError::Transport(failure) => {
            assert_eq!(failure.kind, TransportKind::ReadError);
            assert_eq!(failure.message, "peer closed the connection");
        }
        other => panic!("expected the transport error, got {other:?}"),
    }
    assert_eq!(failure.classes().len(), 1);
}

#[tokio::test]
async fn other_http_errors_raise_even_after_a_terminal_event() {
    let failure = Arc::new(FailurePort::default());
    let mut ports = Ports::default();
    ports.failure = Some(Arc::<FailurePort>::clone(&failure));
    let fixture = Fixture::new(ports, fixed_clock());
    let hooks = fixture.hooks();
    let mut stream = fixture.stream(
        &hooks,
        prefetched(vec![
            payload(COMPLETED),
            transport(TransportKind::HttpError, "the request timed out"),
        ]),
        None,
    );
    stream
        .next_event()
        .await
        .expect("the terminal event yields");
    let error = stream.next_event().await.expect_err("the read fails");
    match error {
        DriverError::Transport(failure) => assert_eq!(failure.kind, TransportKind::HttpError),
        other => panic!("expected the transport error, got {other:?}"),
    }
    assert_eq!(failure.classes().len(), 1);
}

#[tokio::test]
async fn a_framing_failure_raises_an_unclassified_error() {
    let failure = Arc::new(FailurePort::default());
    let mut ports = Ports::default();
    ports.failure = Some(Arc::<FailurePort>::clone(&failure));
    let fixture = Fixture::new(ports, fixed_clock());
    let hooks = fixture.hooks();
    let mut stream = fixture.stream(
        &hooks,
        prefetched(vec![Err(SourceFailure::Decode(
            "the frame could not be read".to_owned(),
        ))]),
        None,
    );
    let error = stream.next_event().await.expect_err("the frame fails");
    assert_eq!(class_of(&error).metadata().status_code, 500);
    assert_eq!(failure.classes().len(), 1);
}

#[test]
fn a_known_completed_event_is_terminal() {
    let event: Event = serde_json::from_str(
        r#"{"type":"response.completed","response":{"id":"resp_1","created_at":1,"object":"response","output":[]}}"#,
    )
    .expect("the fixture event decodes");
    assert!(is_completed(&event));
}

#[test]
fn a_generic_completed_event_is_terminal() {
    let event: Event =
        serde_json::from_str(r#"{"type":"response.completed","response":"not a response body"}"#)
            .expect("the fixture event decodes");
    assert!(is_completed(&event));
}

#[test]
fn a_created_event_is_not_terminal() {
    let event: Event = serde_json::from_str(
        r#"{"type":"response.created","response":{"id":"resp_1","created_at":1,"object":"response","output":[]}}"#,
    )
    .expect("the fixture event decodes");
    assert!(!is_completed(&event));
}

#[test]
fn a_read_timeout_is_not_a_transport_read_error() {
    let failure = classify(HttpError::Timeout);
    assert_eq!(failure.kind, TransportKind::HttpError);
}

#[test]
fn a_body_too_large_failure_is_not_a_transport_read_error() {
    let failure = classify(HttpError::TooLarge { limit: 8 });
    assert_eq!(failure.kind, TransportKind::HttpError);
    assert_eq!(failure.message, "the response body is larger than 8 bytes");
}

#[tokio::test]
async fn a_prefetched_source_yields_each_payload_once_then_ends() {
    let mut source = PrefetchedSource::new(vec![
        Ok(b"first".to_vec()),
        Err(SourceFailure::Decode(
            "the frame could not be read".to_owned(),
        )),
    ]);
    source
        .next_payload()
        .await
        .expect("the first payload is returned")
        .expect("the first payload decodes");
    source
        .next_payload()
        .await
        .expect("the failure payload is returned")
        .expect_err("the failure payload is an error");
    assert!(source.next_payload().await.is_none());
}

mod terminal;
