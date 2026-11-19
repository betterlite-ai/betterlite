#![cfg(test)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_error::error::ErrorClass;
use betterlite_error::error::details::{MidStreamFallbackDetails, TimeoutDetails};
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
use futures_util::StreamExt as _;
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;
use thiserror::Error;

use super::errors::{DriverError, TransportFailure, TransportKind};
use super::live::{Event, LiveStream, LiveStreamSetup, StreamEngine};
use super::source::{BodyStream, PayloadSource, PrefetchedSource, SourceFailure, StreamingSource};
use super::synthetic::{SyntheticSetup, SyntheticStream};
use crate::api::api_boundaries::ApiLimits;
use crate::api::response_synthetic_events::estimate::{
    EstimateRequest, TokenCountRequest, UsageTokenization,
};
use crate::api::response_synthetic_events::stamping::{CostTarget, ResponsesCostCalculator};
use crate::api::responses_stream_events::{ResponsesApiBase, ResponsesEventTransform, StreamTimes};
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

const RESPONSE_FIXTURE: &str = r#"{"id":"resp_a","created_at":1700000000,"status":"completed","output":[{"type":"message","id":"msg_1","status":"completed","role":"assistant","content":[{"type":"output_text","text":"hello world!","annotations":[],"logprobs":null}]}]}"#;

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
    Err(SourceFailure::Transport(TransportFailure::new(
        kind, message,
    )))
}

fn event_json(event: &Event) -> serde_json::Value {
    serde_json::to_value(event).expect("the event serializes")
}

fn event_type(event: &Event) -> String {
    event_json(event)
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn delta(event: &Event) -> String {
    event_json(event)
        .get("delta")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned()
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
    success: Option<Arc<SuccessPort>>,
    failure: Option<Arc<FailurePort>>,
    chunk_hooks: Vec<Arc<ReplacingHook>>,
    cache: Option<Arc<RecordingCache>>,
    cache_handler: Option<Arc<CacheHandler>>,
}

impl Ports {
    fn hooks(
        &self,
        redactor: &'static Redactor,
        clock: Arc<dyn SuccessClock>,
    ) -> ResponsesStreamHooks<'static> {
        let success: Option<Arc<dyn ResponsesStreamSuccessTarget>> = match &self.success {
            Some(port) => {
                let typed: Arc<dyn ResponsesStreamSuccessTarget> = Arc::clone(port);
                Some(typed)
            }
            None => None,
        };
        let failure: Option<Arc<dyn ResponsesStreamFailureTarget>> = match &self.failure {
            Some(port) => {
                let typed: Arc<dyn ResponsesStreamFailureTarget> = Arc::clone(port);
                Some(typed)
            }
            None => None,
        };
        let cache: Option<Arc<dyn ResponsesStreamCacheTarget>> = match &self.cache {
            Some(port) => {
                let typed: Arc<dyn ResponsesStreamCacheTarget> = Arc::clone(port);
                Some(typed)
            }
            None => None,
        };
        let cache_handler: Option<Arc<dyn ResponsesStreamCacheHandler>> = match &self.cache_handler
        {
            Some(port) => {
                let typed: Arc<dyn ResponsesStreamCacheHandler> = Arc::clone(port);
                Some(typed)
            }
            None => None,
        };
        let mut chunk_hooks: Vec<Arc<dyn ChunkDeploymentHook>> = Vec::new();
        for hook in &self.chunk_hooks {
            let typed: Arc<dyn ChunkDeploymentHook> = Arc::clone(hook);
            chunk_hooks.push(typed);
        }
        ResponsesStreamHooks {
            cache,
            cache_handler,
            success,
            failure,
            metadata: None,
            success_hook: None,
            chunk_hooks,
            clock,
            redactor,
        }
    }
}

struct Fixture {
    hooks: ResponsesStreamHooks<'static>,
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
        // the hooks hold the redactor for their whole life, so the fixture leaks one instead of borrowing itself
        let redactor: &'static Redactor = Box::leak(Box::new(redactor()));
        let hooks = ports.hooks(redactor, clock);
        Self {
            hooks,
            logging: logging_object(),
            limits: ApiLimits::default(),
            transform: DecodingTransform,
            tokenization: NoTokenization,
            calculator: NoCost,
            api_base: NoApiBase,
            headers: IndexMap::new(),
        }
    }

    fn stream(
        &self,
        source: Box<dyn PayloadSource>,
        max_streaming_duration_seconds: Option<f64>,
    ) -> LiveStream<'_, DecodingTransform, NoTokenization, NoCost> {
        LiveStream::new(
            StreamEngine {
                transform: Some(&self.transform),
                tokenization: Some(&self.tokenization),
                calculator: Some(&self.calculator),
                hooks: &self.hooks,
                logging: &self.logging,
                details: IndexMap::new(),
                limits: &self.limits,
            },
            LiveStreamSetup {
                model: "gpt-4o",
                custom_llm_provider: Some("openai"),
                litellm_metadata: None,
                request_data: None,
                call_type: Some("responses"),
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

    fn synthetic(
        &self,
        cache_hit: Option<bool>,
        persist_before_logging: bool,
    ) -> SyntheticStream<'_> {
        let response: ResponsesApiResponse =
            serde_json::from_str(RESPONSE_FIXTURE).expect("the response fixture decodes");
        SyntheticStream::build(
            response,
            SyntheticSetup {
                hooks: &self.hooks,
                model: "gpt-4o",
                custom_llm_provider: Some("openai"),
                litellm_metadata: None,
                request_data: None,
                call_type: Some("responses"),
                headers: &self.headers,
                api_base: &self.api_base,
                logging: &self.logging,
                times: StreamTimes {
                    now: timestamp(START),
                    created_timestamp: CREATED_AT,
                },
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

#[tokio::test]
async fn live_sse_delivery_yields_events_and_finishes() {
    let success = Arc::new(SuccessPort::default());
    let failure = Arc::new(FailurePort::default());
    let ports = Ports {
        success: Some(Arc::clone(&success)),
        failure: Some(Arc::clone(&failure)),
        ..Ports::default()
    };
    let fixture = Fixture::new(ports, fixed_clock());
    let mut stream = fixture.stream(
        prefetched(vec![
            payload(CREATED),
            payload(DELTA),
            payload(COMPLETED),
            payload(DONE),
        ]),
        None,
    );
    let first = stream.next_event().await.expect("the created event yields");
    assert_eq!(
        event_type(&first.expect("an event is returned")),
        "response.created"
    );
    let second = stream.next_event().await.expect("the delta yields");
    assert_eq!(delta(&second.expect("an event is returned")), "hi");
    let third = stream
        .next_event()
        .await
        .expect("the terminal event yields");
    assert_eq!(
        event_type(&third.expect("an event is returned")),
        "response.completed"
    );
    assert!(
        stream
            .next_event()
            .await
            .expect("the stream ends")
            .is_none()
    );
    let dispatches = success.dispatches();
    assert_eq!(dispatches.len(), 1);
    assert_eq!(dispatches[0].cache_hit, None);
    assert!(dispatches[0].prefer_async_handlers);
    assert!(success.async_calls().is_empty());
    assert!(failure.classes().is_empty());
}

#[tokio::test]
async fn streaming_source_frames_events_split_across_reads() {
    let body: BodyStream = futures_util::stream::iter(vec![
        Ok(b"data: {\"type\":\"response.cre".to_vec()),
        Ok(b"ated\"}\n\ndata: [DONE]\n".to_vec()),
        Ok(b"\n".to_vec()),
    ])
    .boxed();
    let mut source = StreamingSource::new(body);
    match source.next_payload().await {
        Some(Ok(payload)) => assert_eq!(payload, b"{\"type\":\"response.created\"}".to_vec()),
        _ => panic!("the first event is framed"),
    }
    match source.next_payload().await {
        Some(Ok(payload)) => assert_eq!(payload, DONE.as_bytes().to_vec()),
        _ => panic!("the done marker is framed"),
    }
    assert!(source.next_payload().await.is_none());
}

#[tokio::test]
async fn streaming_source_flushes_an_unterminated_frame_at_eof() {
    let body: BodyStream = futures_util::stream::iter(vec![Ok(b"data: [DONE]".to_vec())]).boxed();
    let mut source = StreamingSource::new(body);
    match source.next_payload().await {
        Some(Ok(payload)) => assert_eq!(payload, DONE.as_bytes().to_vec()),
        _ => panic!("the unterminated frame is flushed"),
    }
    assert!(source.next_payload().await.is_none());
}

#[tokio::test]
async fn early_stream_end_before_any_event_raises_a_fallback() {
    let failure = Arc::new(FailurePort::default());
    let ports = Ports {
        failure: Some(Arc::clone(&failure)),
        ..Ports::default()
    };
    let fixture = Fixture::new(ports, fixed_clock());
    let mut stream = fixture.stream(prefetched(Vec::new()), None);
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
    let mut stream = fixture.stream(prefetched(vec![payload(CREATED)]), None);
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
    let ports = Ports {
        failure: Some(Arc::clone(&failure)),
        ..Ports::default()
    };
    let fixture = Fixture::new(ports, fixed_clock());
    let mut stream = fixture.stream(prefetched(vec![payload(CREATED), payload(DELTA)]), None);
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
    let mut stream = fixture.stream(prefetched(vec![payload(CREATED), payload(DONE)]), None);
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
    let ports = Ports {
        failure: Some(Arc::clone(&failure)),
        ..Ports::default()
    };
    let fixture = Fixture::new(ports, fixed_clock());
    let mut stream = fixture.stream(prefetched(vec![payload(COMPLETED), payload(DONE)]), None);
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
    let ports = Ports {
        failure: Some(Arc::clone(&failure)),
        ..Ports::default()
    };
    let fixture = Fixture::new(ports, fixed_clock());
    let mut stream = fixture.stream(
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
    let ports = Ports {
        failure: Some(Arc::clone(&failure)),
        ..Ports::default()
    };
    let fixture = Fixture::new(ports, fixed_clock());
    let mut stream = fixture.stream(
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
    let ports = Ports {
        failure: Some(Arc::clone(&failure)),
        ..Ports::default()
    };
    let fixture = Fixture::new(ports, fixed_clock());
    let mut stream = fixture.stream(
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
    let ports = Ports {
        failure: Some(Arc::clone(&failure)),
        ..Ports::default()
    };
    let fixture = Fixture::new(ports, fixed_clock());
    let mut stream = fixture.stream(
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

#[rstest]
#[case::rate_limit(RATE_LIMITED, 429, true)]
#[case::server_error(SERVER_ERROR, 500, true)]
#[case::bad_request(BAD_REQUEST, 400, false)]
#[tokio::test]
async fn terminal_error_events_raise_before_the_deployment_hook(
    #[case] event: &str,
    #[case] status: u16,
    #[case] falls_back: bool,
) {
    let hook =
        ReplacingHook::new(serde_json::from_str(COMPLETED).expect("the replacement decodes"));
    let failure = Arc::new(FailurePort::default());
    let ports = Ports {
        failure: Some(Arc::clone(&failure)),
        chunk_hooks: vec![Arc::clone(&hook)],
        ..Ports::default()
    };
    let fixture = Fixture::new(ports, fixed_clock());
    let mut stream = fixture.stream(prefetched(vec![payload(CREATED), payload(event)]), None);
    stream.next_event().await.expect("the created event yields");
    let error = stream
        .next_event()
        .await
        .expect_err("the error event raises");
    assert_eq!(status, class_of(&error).metadata().status_code);
    match class_of(&error) {
        ErrorClass::MidStreamFallback(details) => {
            assert!(falls_back);
            assert_eq!(details.original_status.0, status);
        }
        other => assert!(!falls_back, "unexpected class {other:?}"),
    }
    assert_eq!(hook.calls(), vec!["response.created".to_owned()]);
    assert_eq!(failure.classes().len(), 1);
}

#[tokio::test]
async fn the_deployment_hook_decides_whether_output_started() {
    let replacement: Event = serde_json::from_str(DELTA).expect("the replacement decodes");
    let hook = ReplacingHook::new(replacement);
    let ports = Ports {
        chunk_hooks: vec![Arc::clone(&hook)],
        ..Ports::default()
    };
    let fixture = Fixture::new(ports, fixed_clock());
    let mut stream = fixture.stream(prefetched(vec![payload(CREATED)]), None);
    let yielded = stream
        .next_event()
        .await
        .expect("the replaced event yields")
        .expect("an event is returned");
    assert_eq!(delta(&yielded), "hi");
    let error = stream.next_event().await.expect_err("the stream fails");
    connection_message(&error);
    assert_eq!(hook.calls(), vec!["response.created".to_owned()]);
}

#[tokio::test]
async fn a_queued_event_does_not_start_the_output() {
    let fixture = Fixture::new(Ports::default(), fixed_clock());
    let mut stream = fixture.stream(prefetched(vec![payload(QUEUED)]), None);
    let queued = stream
        .next_event()
        .await
        .expect("the queued event yields")
        .expect("an event is returned");
    assert_eq!(event_type(&queued), "response.queued");
    let error = stream.next_event().await.expect_err("the stream fails");
    assert_eq!(fallback(&error).original_status.0, 500);
}

#[tokio::test]
async fn an_in_progress_event_does_not_start_the_output() {
    let fixture = Fixture::new(Ports::default(), fixed_clock());
    let mut stream = fixture.stream(prefetched(vec![payload(IN_PROGRESS)]), None);
    let started = stream
        .next_event()
        .await
        .expect("the event yields")
        .expect("an event is returned");
    assert_eq!(event_type(&started), "response.in_progress");
    let error = stream.next_event().await.expect_err("the stream fails");
    assert_eq!(fallback(&error).original_status.0, 500);
}

#[tokio::test]
async fn undecodable_payloads_are_skipped() {
    let fixture = Fixture::new(Ports::default(), fixed_clock());
    let mut stream = fixture.stream(
        prefetched(vec![payload("not json"), payload(COMPLETED)]),
        None,
    );
    let event = stream
        .next_event()
        .await
        .expect("the terminal event yields")
        .expect("an event is returned");
    assert_eq!(event_type(&event), "response.completed");
}

#[tokio::test]
async fn streaming_duration_limit_blocks_the_first_read() {
    let failure = Arc::new(FailurePort::default());
    let ports = Ports {
        failure: Some(Arc::clone(&failure)),
        ..Ports::default()
    };
    let fixture = Fixture::new(ports, scripted_clock(&[LATE]));
    let (source, polls) = CountingSource::new(&[CREATED]);
    let mut stream = fixture.stream(Box::new(source), Some(10.0));
    let error = stream.next_event().await.expect_err("the stream times out");
    assert_eq!(
        timeout_details(&error).context.message,
        "litellm.Timeout: Stream exceeded max streaming duration of 10s (elapsed 20.0s)"
    );
    assert_eq!(poll_count(&polls), 0);
    assert_eq!(failure.classes().len(), 1);
}

#[tokio::test]
async fn streaming_duration_limit_stops_after_a_received_event() {
    let fixture = Fixture::new(Ports::default(), scripted_clock(&[START, LATE]));
    let (source, polls) = CountingSource::new(&[CREATED]);
    let mut stream = fixture.stream(Box::new(source), Some(10.0));
    let error = stream.next_event().await.expect_err("the stream times out");
    assert_eq!(timeout_details(&error).original_status.0, 408);
    assert_eq!(poll_count(&polls), 1);
}

#[test]
fn sync_iteration_runs_failure_handlers_without_a_runtime() {
    let failure = Arc::new(FailurePort::default());
    let ports = Ports {
        failure: Some(Arc::clone(&failure)),
        ..Ports::default()
    };
    let fixture = Fixture::new(ports, fixed_clock());
    let mut stream = fixture.stream(prefetched(vec![payload(CREATED)]), None);
    stream
        .next_event_blocking()
        .expect("the created event yields")
        .expect("an event is returned");
    let error = stream
        .next_event_blocking()
        .expect_err("the stream ends early");
    connection_message(&error);
    assert_eq!(failure.classes().len(), 1);
}

#[test]
fn sync_iteration_logs_a_completed_response_inline() {
    let success = Arc::new(SuccessPort::default());
    let ports = Ports {
        success: Some(Arc::clone(&success)),
        ..Ports::default()
    };
    let fixture = Fixture::new(ports, fixed_clock());
    let mut stream = fixture.stream(prefetched(vec![payload(COMPLETED)]), None);
    stream
        .next_event_blocking()
        .expect("the terminal event yields")
        .expect("an event is returned");
    assert_eq!(success.async_calls().len(), 1);
    assert_eq!(success.sync_calls().len(), 1);
    assert!(success.dispatches().is_empty());
}

#[tokio::test]
async fn fake_response_iteration_slices_text_into_chunks() {
    let success = Arc::new(SuccessPort::default());
    let cache = Arc::new(RecordingCache::default());
    let ports = Ports {
        success: Some(Arc::clone(&success)),
        cache: Some(Arc::clone(&cache)),
        cache_handler: Some(Arc::new(caching_handler())),
        ..Ports::default()
    };
    let fixture = Fixture::new(ports, fixed_clock());
    let mut stream = fixture.synthetic(None, true);
    let mut types = Vec::new();
    let mut deltas = Vec::new();
    while let Some(event) = stream
        .next_event_async()
        .await
        .expect("the synthetic event yields")
    {
        if event_type(&event) == "response.output_text.delta" {
            deltas.push(delta(&event));
        }
        types.push(event_type(&event));
    }
    assert_eq!(
        deltas,
        vec!["hello".to_owned(), " worl".to_owned(), "d!".to_owned()]
    );
    assert_eq!(types.last().map(String::as_str), Some("response.completed"));
    stream.drain_logging().await;
    let dispatches = success.dispatches();
    assert_eq!(dispatches.len(), 1);
    assert_eq!(dispatches[0].cache_hit, None);
    assert_eq!(cache.writes().len(), 1);
}

#[test]
fn cached_response_iteration_marks_cache_hit_and_skips_persistence() {
    let success = Arc::new(SuccessPort::default());
    let cache = Arc::new(RecordingCache::default());
    let ports = Ports {
        success: Some(Arc::clone(&success)),
        cache: Some(Arc::clone(&cache)),
        cache_handler: Some(Arc::new(caching_handler())),
        ..Ports::default()
    };
    let fixture = Fixture::new(ports, fixed_clock());
    let mut stream = fixture.synthetic(Some(true), false);
    let mut deltas = Vec::new();
    loop {
        let event = stream.next_event().expect("the synthetic event yields");
        let Some(event) = event else { break };
        if event_type(&event) == "response.output_text.delta" {
            deltas.push(delta(&event));
        }
    }
    assert_eq!(
        deltas,
        vec!["hello".to_owned(), " worl".to_owned(), "d!".to_owned()]
    );
    assert_eq!(success.async_calls().len(), 1);
    assert_eq!(success.sync_calls().len(), 1);
    assert_eq!(success.async_calls()[0].cache_hit, Some(true));
    assert!(cache.writes().is_empty());
}
