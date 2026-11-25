#![cfg(test)]

use std::io::{self, Result as IoResult, Write};
use std::mem::take;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{ThreadId, current};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use futures_util::stream::{self, Stream};
use indexmap::IndexMap;
use tokio_util::task::TaskTracker;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt;
use tracing_subscriber::fmt::MakeWriter;

use crate::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use crate::logging_state::types::{CallbackPolicy, DynamicCallbacks, RequestLoggingFlags};
use crate::logging_state::{LoggingInit, LoggingState};
use crate::logging_success::ports::SuccessClock;
use crate::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use crate::passthrough_logging_state::logger::PassThroughEndpointLogging;
use crate::passthrough_logging_state::response::{PassThroughResponse, PassThroughResponseObject};
use crate::passthrough_stream_logging::build::StreamLoggingInput;
use crate::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamLoggingState, StreamSuccessCall, StreamSuccessDispatch,
};
use crate::passthrough_stream_logging::router::{AnthropicFrameProbe, StreamLoggingRouter};
use crate::passthrough_stream_logging::tinyfish::{TinyfishCollectedChunks, TinyfishStreamBilling};
use crate::passthrough_stream_logging::types::{
    CollectedChunksCall, CollectedChunksHandler, EndpointType, HandlerError, HandlerOutcome,
    ProviderChunkHandlers,
};
use crate::reservation_binding::{ReservationBinder, stamp_callback_bound};
use crate::worker::{Admission, Rejection};

use super::chunk_processor::ChunkProcessorCall;
use super::cost_injection::StreamCostInjection;
use super::failure::{StreamFailure, StreamFailureDispatch};
use super::lifecycle::StreamLifecycle;
use super::partial_usage::AnthropicPartialUsage;
use super::ports::{
    DeferredStreamLogging, StreamJobKind, StreamLoggingJob, StreamLoggingQueue, StreamRoute,
    StreamWork,
};

pub(super) fn time(text: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(text).unwrap()
}

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("valid JSON literal")
}

pub(super) fn state(details: &[(&str, Json)]) -> LoggingState {
    let mut context = CorrelationContext::default();
    let setup = CorrelationSetup {
        context: &mut context,
        settings: CorrelationSettings::default(),
    };
    let mut logging = LoggingState::new(
        logging_init(),
        setup,
        &Redactor::new(16).expect("valid redactor"),
        || "generated-trace".to_owned(),
        || Duration::ZERO,
    )
    .expect("valid logging state");
    for (key, value) in details {
        logging
            .model_call_details
            .insert((*key).to_owned(), value.clone());
    }
    logging
}

fn logging_init() -> LoggingInit {
    LoggingInit {
        model: "gpt-4o".to_owned(),
        call_type: "pass_through_endpoint".to_owned(),
        start_time: time("2024-01-01T00:00:00+00:00"),
        litellm_call_id: "call-1".to_owned(),
        function_id: "function-1".to_owned(),
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
    }
}

pub(super) fn object(
    details: &[(&str, Json)],
    litellm_params: IndexMap<String, Json>,
) -> Arc<StreamLoggingObject> {
    let mut logging = state(details);
    logging.litellm_params = litellm_params;
    Arc::new(StreamLoggingObject {
        slot: Mutex::new(Some(StreamLoggingState {
            logging,
            correlation: CorrelationContext::default(),
        })),
        dispatch: Arc::new(NoSuccessDispatch),
    })
}

pub(super) fn passthrough() -> Arc<PassThroughEndpointLogging> {
    Arc::new(PassThroughEndpointLogging {
        injected_log_dispatch: None,
    })
}

pub(super) fn reservation_params() -> IndexMap<String, Json> {
    let mut metadata = IndexMap::new();
    let mut reservation = IndexMap::new();
    reservation.insert("id".to_owned(), Json::Text("reservation-1".to_owned()));
    metadata.insert(
        "user_api_key_budget_reservation".to_owned(),
        Json::Object(reservation),
    );
    let mut params = IndexMap::new();
    params.insert("metadata".to_owned(), Json::Object(metadata));
    params
}

pub(super) fn model_details(logging_obj: &StreamLoggingObject) -> IndexMap<String, Json> {
    logging_obj
        .read(&mut |state| state.model_call_details.clone())
        .expect("the logging state is present")
}

pub(super) fn litellm_params(logging_obj: &StreamLoggingObject) -> IndexMap<String, Json> {
    logging_obj
        .read(&mut |state| state.litellm_params.clone())
        .expect("the logging state is present")
}

pub(super) fn callback_bound(logging_obj: &StreamLoggingObject) -> Option<Json> {
    let params = litellm_params(logging_obj);
    let Json::Object(metadata) = params.get("metadata")? else {
        return None;
    };
    let Json::Object(reservation) = metadata.get("user_api_key_budget_reservation")? else {
        return None;
    };
    reservation.get("callback_bound").cloned()
}

pub(super) fn recorded_usage(logging_obj: &StreamLoggingObject) -> Option<(i64, i64, i64)> {
    let details = model_details(logging_obj);
    let Some(Json::Object(usage)) = details.get("combined_usage_object") else {
        return None;
    };
    Some((
        usage_token(usage, "prompt_tokens")?,
        usage_token(usage, "completion_tokens")?,
        usage_token(usage, "total_tokens")?,
    ))
}

fn usage_token(usage: &IndexMap<String, Json>, key: &str) -> Option<i64> {
    let Some(Json::Number(number)) = usage.get(key) else {
        return None;
    };
    number.as_i64()
}

pub(super) fn completion_start_time(logging_obj: &StreamLoggingObject) -> Option<String> {
    let value = logging_obj
        .read(&mut |state| state.completion_start_time)
        .flatten()?;
    Some(value.to_rfc3339())
}

pub(super) fn response_cost(logging_obj: &StreamLoggingObject) -> Option<Json> {
    model_details(logging_obj).get("response_cost").cloned()
}

fn usage_handler_outcome(
    prompt: i64,
    completion: i64,
    total: i64,
    cost: Option<&str>,
) -> HandlerOutcome {
    let mut kwargs = PassThroughLoggingKwargs::default();
    if let Some(amount) = cost {
        let number = amount
            .parse::<serde_json::Number>()
            .expect("a decimal response cost");
        kwargs
            .entries
            .insert("response_cost".to_owned(), Json::Number(number));
    }
    HandlerOutcome {
        result: Some(PassThroughResponse::ModelResponse(model_response_body(
            prompt, completion, total,
        ))),
        kwargs,
    }
}

pub(super) fn usage_outcome(
    prompt: i64,
    completion: i64,
    total: i64,
    cost: &str,
) -> HandlerOutcome {
    usage_handler_outcome(prompt, completion, total, Some(cost))
}

pub(super) fn usage_without_cost_outcome(
    prompt: i64,
    completion: i64,
    total: i64,
) -> HandlerOutcome {
    usage_handler_outcome(prompt, completion, total, None)
}

fn model_response_body(prompt: i64, completion: i64, total: i64) -> Json {
    json(&format!(
        r#"{{"id":"chatcmpl-1","usage":{{"prompt_tokens":{prompt},"completion_tokens":{completion},"total_tokens":{total}}}}}"#
    ))
}

pub(super) fn standard_outcome(text: &str) -> HandlerOutcome {
    let body = json(&format!(r#"{{"response":"{text}"}}"#));
    HandlerOutcome {
        result: Some(PassThroughResponse::Standard(PassThroughResponseObject {
            response: body,
        })),
        kwargs: PassThroughLoggingKwargs::default(),
    }
}

#[derive(Clone)]
pub(super) struct RecordedHandlerCall {
    pub(super) url_route: String,
    pub(super) request_body: Option<Json>,
    pub(super) endpoint_type: EndpointType,
    pub(super) start_time: DateTime<FixedOffset>,
    pub(super) end_time: DateTime<FixedOffset>,
    pub(super) lines: Vec<String>,
    pub(super) model: Option<String>,
    pub(super) thread: ThreadId,
}

#[derive(Default)]
pub(super) struct RecordingHandler {
    pub(super) calls: Mutex<Vec<RecordedHandlerCall>>,
    pub(super) reply: Mutex<Option<Result<HandlerOutcome, HandlerError>>>,
}

impl RecordingHandler {
    pub(super) fn returning(reply: Result<HandlerOutcome, HandlerError>) -> Arc<Self> {
        let handler = Self::default();
        *handler.reply.lock().unwrap_or_else(PoisonError::into_inner) = Some(reply);
        Arc::new(handler)
    }

    pub(super) fn called(&self) -> usize {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

impl CollectedChunksHandler for RecordingHandler {
    fn handle(&self, call: &CollectedChunksCall<'_>) -> Result<HandlerOutcome, HandlerError> {
        let recorded = RecordedHandlerCall {
            url_route: call.url_route.to_owned(),
            request_body: call.request_body.cloned(),
            endpoint_type: call.endpoint_type,
            start_time: call.start_time,
            end_time: call.end_time,
            lines: call.lines.to_vec(),
            model: call.model.map(str::to_owned),
            thread: current().id(),
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(recorded);
        self.reply
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .unwrap_or_else(|| Ok(HandlerOutcome::default()))
    }
}

pub(super) fn providers(handler: &Arc<dyn CollectedChunksHandler>) -> Arc<ProviderChunkHandlers> {
    Arc::new(ProviderChunkHandlers {
        anthropic: Arc::clone(handler),
        vertex: Arc::clone(handler),
        gemini: Arc::clone(handler),
        openai: Arc::clone(handler),
    })
}

pub(super) struct StepClock {
    values: Mutex<Vec<DateTime<FixedOffset>>>,
    calls: Mutex<usize>,
}

impl StepClock {
    pub(super) fn new(values: &[&str]) -> Arc<Self> {
        let mut parsed = Vec::with_capacity(values.len());
        for value in values {
            parsed.push(time(value));
        }
        Arc::new(Self {
            values: Mutex::new(parsed),
            calls: Mutex::new(0),
        })
    }

    pub(super) fn calls(&self) -> usize {
        *self.calls.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl SuccessClock for StepClock {
    fn now(&self) -> DateTime<FixedOffset> {
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        *calls = calls.saturating_add(1);
        drop(calls);
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        if values.len() > 1 {
            values.remove(0)
        } else {
            *values.first().expect("a scripted clock value")
        }
    }
}

#[derive(Default)]
pub(super) struct RecordingQueue {
    pub(super) jobs: Mutex<Vec<StreamLoggingJob>>,
    pub(super) rejecting: bool,
}

impl RecordingQueue {
    pub(super) fn kinds(&self) -> Vec<StreamJobKind> {
        let jobs = self.jobs.lock().unwrap_or_else(PoisonError::into_inner);
        job_kinds(&jobs)
    }

    pub(super) fn take_all(&self) -> Vec<StreamLoggingJob> {
        take(&mut *self.jobs.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

impl StreamLoggingQueue for RecordingQueue {
    fn submit(&self, job: StreamLoggingJob) -> Admission {
        if self.rejecting {
            return Admission::Rejected(Rejection::QueueFull);
        }
        self.jobs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(job);
        Admission::Accepted
    }
}

#[derive(Default)]
pub(super) struct RecordingDeferred {
    pub(super) parked: Mutex<Vec<StreamLoggingJob>>,
}

impl RecordingDeferred {
    pub(super) fn kinds(&self) -> Vec<StreamJobKind> {
        let jobs = self.parked.lock().unwrap_or_else(PoisonError::into_inner);
        job_kinds(&jobs)
    }

    pub(super) fn take_all(&self) -> Vec<StreamLoggingJob> {
        take(&mut *self.parked.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

fn job_kinds(jobs: &[StreamLoggingJob]) -> Vec<StreamJobKind> {
    let mut kinds = Vec::with_capacity(jobs.len());
    for job in jobs {
        kinds.push(job.kind);
    }
    kinds
}

impl DeferredStreamLogging for RecordingDeferred {
    fn park(&self, job: StreamLoggingJob) {
        self.parked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(job);
    }
}

#[derive(Clone, Debug)]
pub(super) struct RecordedRoute {
    pub(super) url_route: String,
    pub(super) request_body: Option<Json>,
    pub(super) endpoint_type: EndpointType,
    pub(super) start_time: DateTime<FixedOffset>,
    pub(super) end_time: DateTime<FixedOffset>,
    pub(super) raw_bytes: Vec<Vec<u8>>,
    pub(super) model: Option<String>,
}

#[derive(Default)]
pub(super) struct RecordingRoute {
    pub(super) inputs: Mutex<Vec<RecordedRoute>>,
}

impl RecordingRoute {
    pub(super) fn calls(&self) -> Vec<RecordedRoute> {
        self.inputs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl StreamRoute for RecordingRoute {
    fn route(self: Arc<Self>, input: StreamLoggingInput) -> StreamWork {
        Box::pin(async move {
            let recorded = RecordedRoute {
                url_route: input.url_route,
                request_body: input.request_body,
                endpoint_type: input.endpoint_type,
                start_time: input.start_time,
                end_time: input.end_time,
                raw_bytes: input.raw_bytes,
                model: input.model,
            };
            self.inputs
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(recorded);
        })
    }
}

#[derive(Default)]
pub(super) struct RecordingCostInjection {
    pub(super) calls: Mutex<Vec<(Vec<u8>, String)>>,
}

impl RecordingCostInjection {
    pub(super) fn calls(&self) -> Vec<(Vec<u8>, String)> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl StreamCostInjection for RecordingCostInjection {
    fn inject(
        &self,
        frames: &[u8],
        model_name: &str,
        _logging_obj: &StreamLoggingObject,
    ) -> Vec<u8> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((frames.to_vec(), model_name.to_owned()));
        let mut output = b"injected:".to_vec();
        output.extend_from_slice(frames);
        output
    }
}

#[derive(Default)]
pub(super) struct RecordingAnthropic {
    pub(super) calls: Mutex<Vec<(Json, Vec<Vec<u8>>)>>,
}

impl AnthropicPartialUsage for RecordingAnthropic {
    fn record_partial_usage(
        &self,
        _logging_obj: &StreamLoggingObject,
        request_body: &Json,
        all_chunks: &[Vec<u8>],
    ) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((request_body.clone(), all_chunks.to_vec()));
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RecordedFailure {
    pub(super) class: String,
    pub(super) message: String,
    pub(super) traceback: String,
}

#[derive(Default)]
pub(super) struct RecordingFailureDispatch {
    pub(super) failures: Mutex<Vec<RecordedFailure>>,
}

impl RecordingFailureDispatch {
    pub(super) fn failures(&self) -> Vec<RecordedFailure> {
        self.failures
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl StreamFailureDispatch for RecordingFailureDispatch {
    fn dispatch_failure(
        &self,
        _logging_obj: Arc<StreamLoggingObject>,
        failure: &StreamFailure,
    ) -> StreamWork {
        self.failures
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(recorded(failure));
        Box::pin(async {})
    }
}

pub(super) fn recorded(failure: &StreamFailure) -> RecordedFailure {
    RecordedFailure {
        class: failure.class().to_owned(),
        message: failure.message().to_owned(),
        traceback: failure.traceback().to_owned(),
    }
}

#[derive(Default)]
pub(super) struct RecordingBinder {
    pub(super) calls: Mutex<usize>,
}

impl RecordingBinder {
    pub(super) fn calls(&self) -> usize {
        *self.calls.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl ReservationBinder for RecordingBinder {
    fn bind(&self, litellm_params: &mut IndexMap<String, Json>) {
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        *calls = calls.saturating_add(1);
        drop(calls);
        stamp_callback_bound(litellm_params, true);
    }
}

#[derive(Default)]
pub(super) struct RecordingTinyfish {
    pub(super) poller: bool,
    pub(super) run_id: Option<String>,
    pub(super) scans: Mutex<Vec<Vec<u8>>>,
    pub(super) started: Mutex<Vec<(String, DateTime<FixedOffset>)>>,
}

impl RecordingTinyfish {
    pub(super) fn started(&self) -> Vec<(String, DateTime<FixedOffset>)> {
        self.started
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl TinyfishStreamBilling for RecordingTinyfish {
    fn sse_poller_spawned(&self, _logging_obj: &StreamLoggingObject) -> bool {
        self.poller
    }

    fn run_id_from_sse_frames(&self, frames: &[u8]) -> Option<String> {
        self.scans
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(frames.to_vec());
        self.run_id.clone()
    }

    fn start_sse_run_billing(
        &self,
        run_id: &str,
        _logging_obj: &StreamLoggingObject,
        start_time: DateTime<FixedOffset>,
    ) {
        self.started
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((run_id.to_owned(), start_time));
    }

    async fn handle_logging_tinyfish_collected_chunks(
        &self,
        _call: &TinyfishCollectedChunks<'_>,
    ) -> Result<HandlerOutcome, HandlerError> {
        Ok(HandlerOutcome::default())
    }
}

#[derive(Default)]
pub(super) struct FakeProbe;

impl AnthropicFrameProbe for FakeProbe {
    fn is_message_stop(&self, _frames: &[u8]) -> bool {
        false
    }

    fn is_provider_error(&self, _frames: &[u8]) -> bool {
        false
    }
}

pub(super) struct NoSuccessDispatch;

#[async_trait]
impl StreamSuccessDispatch for NoSuccessDispatch {
    async fn dispatch_success(&self, _call: StreamSuccessCall<'_>) {}
}

pub(super) struct Harness {
    pub(super) clock: Arc<StepClock>,
    pub(super) queue: Arc<RecordingQueue>,
    pub(super) deferred: Option<Arc<RecordingDeferred>>,
    pub(super) binder: Arc<RecordingBinder>,
    pub(super) route: Arc<RecordingRoute>,
    pub(super) cost_injection: Arc<RecordingCostInjection>,
    pub(super) anthropic: Arc<RecordingAnthropic>,
    pub(super) failure_dispatch: Arc<RecordingFailureDispatch>,
    pub(super) tinyfish: Arc<RecordingTinyfish>,
    pub(super) handler: Arc<RecordingHandler>,
    pub(super) providers: Arc<ProviderChunkHandlers>,
    pub(super) probe: Arc<FakeProbe>,
    pub(super) injection_override: Option<Arc<dyn StreamCostInjection>>,
}

impl Harness {
    pub(super) fn new(values: &[&str]) -> Self {
        let handler = RecordingHandler::returning(Ok(HandlerOutcome::default()));
        let collected: Arc<dyn CollectedChunksHandler> = Arc::<RecordingHandler>::clone(&handler);
        Self {
            clock: StepClock::new(values),
            queue: Arc::new(RecordingQueue::default()),
            deferred: None,
            binder: Arc::new(RecordingBinder::default()),
            route: Arc::new(RecordingRoute::default()),
            cost_injection: Arc::new(RecordingCostInjection::default()),
            anthropic: Arc::new(RecordingAnthropic::default()),
            failure_dispatch: Arc::new(RecordingFailureDispatch::default()),
            tinyfish: Arc::new(RecordingTinyfish::default()),
            providers: providers(&collected),
            handler,
            probe: Arc::new(FakeProbe),
            injection_override: None,
        }
    }

    pub(super) fn with_handler_reply(
        mut self,
        reply: Result<HandlerOutcome, HandlerError>,
    ) -> Self {
        self.handler = RecordingHandler::returning(reply);
        let collected: Arc<dyn CollectedChunksHandler> =
            Arc::<RecordingHandler>::clone(&self.handler);
        self.providers = providers(&collected);
        self
    }

    pub(super) fn with_injection(mut self, injection: Arc<dyn StreamCostInjection>) -> Self {
        self.injection_override = Some(injection);
        self
    }

    pub(super) fn with_deferred(mut self) -> Self {
        self.deferred = Some(Arc::new(RecordingDeferred::default()));
        self
    }

    pub(super) fn with_rejecting_queue(mut self) -> Self {
        self.queue = Arc::new(RecordingQueue {
            jobs: Mutex::new(Vec::new()),
            rejecting: true,
        });
        self
    }

    pub(super) fn with_tinyfish(mut self, poller: bool, run_id: Option<&str>) -> Self {
        self.tinyfish = Arc::new(RecordingTinyfish {
            poller,
            run_id: run_id.map(str::to_owned),
            scans: Mutex::new(Vec::new()),
            started: Mutex::new(Vec::new()),
        });
        self
    }

    pub(super) fn deferred_handle(&self) -> &Arc<RecordingDeferred> {
        self.deferred
            .as_ref()
            .expect("the harness has deferred logging")
    }

    pub(super) fn lifecycle(&self, cost_injection_enabled: bool) -> Arc<StreamLifecycle> {
        let cost_injection: Arc<dyn StreamCostInjection> =
            if let Some(injection) = &self.injection_override {
                Arc::clone(injection)
            } else {
                Arc::<RecordingCostInjection>::clone(&self.cost_injection)
            };
        let mut lifecycle = StreamLifecycle {
            tasks: TaskTracker::new(),
            router: Arc::new(StreamLoggingRouter {
                providers: Arc::clone(&self.providers),
                frames: Arc::<FakeProbe>::clone(&self.probe),
                tinyfish: Arc::<RecordingTinyfish>::clone(&self.tinyfish),
            }),
            passthrough: passthrough(),
            route: Arc::<RecordingRoute>::clone(&self.route),
            cost_injection,
            anthropic_partial_usage: Arc::<RecordingAnthropic>::clone(&self.anthropic),
            failure_dispatch: Arc::<RecordingFailureDispatch>::clone(&self.failure_dispatch),
            queue: Arc::<RecordingQueue>::clone(&self.queue),
            binder: Arc::<RecordingBinder>::clone(&self.binder),
            deferred: None,
            clock: Arc::<StepClock>::clone(&self.clock),
            cost_injection_enabled,
        };
        if let Some(parked) = &self.deferred {
            let job: Arc<dyn DeferredStreamLogging> = Arc::<RecordingDeferred>::clone(parked);
            lifecycle.deferred = Some(job);
        }
        Arc::new(lifecycle)
    }
}

pub(super) fn processor_call(
    lifecycle: Arc<StreamLifecycle>,
    logging_obj: Arc<StreamLoggingObject>,
    endpoint_type: EndpointType,
) -> ChunkProcessorCall {
    ChunkProcessorCall {
        lifecycle,
        logging_obj,
        endpoint_type,
        start_time: time("2024-01-01T00:00:00+00:00"),
        url_route: "/v1/messages".to_owned(),
        request_body: None,
        status_code: 200,
    }
}

pub(super) fn ok_chunks(
    items: Vec<Vec<u8>>,
) -> impl Stream<Item = Result<Vec<u8>, StreamFailure>> + Send + 'static {
    stream::iter(items.into_iter().map(Ok))
}

pub(super) fn failing_chunks(
    items: Vec<Vec<u8>>,
    failure: StreamFailure,
) -> impl Stream<Item = Result<Vec<u8>, StreamFailure>> + Send + 'static {
    let mut all: Vec<Result<Vec<u8>, StreamFailure>> = items.into_iter().map(Ok).collect();
    all.push(Err(failure));
    stream::iter(all)
}

pub(super) fn failure() -> StreamFailure {
    StreamFailure::upstream(
        "ScriptedReadError".to_owned(),
        "the upstream read failed".to_owned(),
        "Traceback: scripted".to_owned(),
        Arc::new(io::Error::other("the upstream read failed")),
    )
}

#[derive(Clone, Default)]
pub(super) struct Captured(pub(super) Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(super) fn text(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).expect("the captured log is UTF-8")
    }
}

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for Captured {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

pub(super) fn capture_logs() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::WARN)
        .with_writer(move || writer.clone())
        .finish();
    (captured, set_default(subscriber))
}
