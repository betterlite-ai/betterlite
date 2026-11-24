#![cfg(test)]

use std::io::{Result as IoResult, Write};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{ThreadId, current};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use futures_util::stream::{self, Stream};
use indexmap::IndexMap;
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

pub fn time(text: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(text).unwrap()
}

pub fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("valid JSON literal")
}

pub fn state(details: &[(&str, Json)]) -> LoggingState {
    let init = LoggingInit {
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
    };
    let mut context = CorrelationContext::default();
    let setup = CorrelationSetup {
        context: &mut context,
        settings: CorrelationSettings::default(),
    };
    let mut logging = LoggingState::new(
        init,
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

pub fn object(
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

pub fn passthrough() -> Arc<PassThroughEndpointLogging> {
    Arc::new(PassThroughEndpointLogging {
        injected_log_dispatch: None,
    })
}

pub fn reservation_params() -> IndexMap<String, Json> {
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

pub fn model_details(logging_obj: &StreamLoggingObject) -> IndexMap<String, Json> {
    logging_obj
        .read(&mut |state| state.model_call_details.clone())
        .expect("the logging state is present")
}

pub fn litellm_params(logging_obj: &StreamLoggingObject) -> IndexMap<String, Json> {
    logging_obj
        .read(&mut |state| state.litellm_params.clone())
        .expect("the logging state is present")
}

pub fn callback_bound(logging_obj: &StreamLoggingObject) -> Option<Json> {
    let params = litellm_params(logging_obj);
    let Json::Object(metadata) = params.get("metadata")? else {
        return None;
    };
    let Json::Object(reservation) = metadata.get("user_api_key_budget_reservation")? else {
        return None;
    };
    reservation.get("callback_bound").cloned()
}

pub fn recorded_usage(logging_obj: &StreamLoggingObject) -> Option<(i64, i64, i64)> {
    let details = model_details(logging_obj);
    let Some(Json::Object(usage)) = details.get("combined_usage_object") else {
        return None;
    };
    let read = |key: &str| match usage.get(key) {
        Some(Json::Number(number)) => number.as_i64(),
        _ => None,
    };
    Some((
        read("prompt_tokens")?,
        read("completion_tokens")?,
        read("total_tokens")?,
    ))
}

pub fn completion_start_time(logging_obj: &StreamLoggingObject) -> Option<String> {
    logging_obj
        .read(&mut |state| state.completion_start_time.map(|value| value.to_rfc3339()))
        .flatten()
}

pub fn response_cost(logging_obj: &StreamLoggingObject) -> Option<Json> {
    model_details(logging_obj).get("response_cost").cloned()
}

pub fn usage_outcome(prompt: i64, completion: i64, total: i64, cost: f64) -> HandlerOutcome {
    let mut kwargs = PassThroughLoggingKwargs::default();
    let number = serde_json::Number::from_f64(cost).expect("a finite response cost");
    kwargs
        .entries
        .insert("response_cost".to_owned(), Json::Number(number));
    HandlerOutcome {
        result: Some(PassThroughResponse::ModelResponse(model_response_body(
            prompt, completion, total,
        ))),
        kwargs,
    }
}

pub fn usage_without_cost_outcome(prompt: i64, completion: i64, total: i64) -> HandlerOutcome {
    HandlerOutcome {
        result: Some(PassThroughResponse::ModelResponse(model_response_body(
            prompt, completion, total,
        ))),
        kwargs: PassThroughLoggingKwargs::default(),
    }
}

fn model_response_body(prompt: i64, completion: i64, total: i64) -> Json {
    json(&format!(
        r#"{{"id":"chatcmpl-1","usage":{{"prompt_tokens":{prompt},"completion_tokens":{completion},"total_tokens":{total}}}}}"#
    ))
}

pub fn standard_outcome(text: &str) -> HandlerOutcome {
    let body = json(&format!(r#"{{"response":"{text}"}}"#));
    HandlerOutcome {
        result: Some(PassThroughResponse::Standard(PassThroughResponseObject {
            response: body,
        })),
        kwargs: PassThroughLoggingKwargs::default(),
    }
}

#[derive(Clone)]
pub struct RecordedHandlerCall {
    pub url_route: String,
    pub request_body: Option<Json>,
    pub endpoint_type: EndpointType,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
    pub lines: Vec<String>,
    pub model: Option<String>,
    pub thread: ThreadId,
}

#[derive(Default)]
pub struct RecordingHandler {
    pub calls: Mutex<Vec<RecordedHandlerCall>>,
    pub reply: Mutex<Option<Result<HandlerOutcome, HandlerError>>>,
}

impl RecordingHandler {
    pub fn returning(reply: Result<HandlerOutcome, HandlerError>) -> Arc<Self> {
        let handler = Self::default();
        *handler.reply.lock().unwrap_or_else(PoisonError::into_inner) = Some(reply);
        Arc::new(handler)
    }

    pub fn called(&self) -> usize {
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

pub fn providers(handler: &Arc<dyn CollectedChunksHandler>) -> Arc<ProviderChunkHandlers> {
    Arc::new(ProviderChunkHandlers {
        anthropic: Arc::clone(handler),
        vertex: Arc::clone(handler),
        gemini: Arc::clone(handler),
        openai: Arc::clone(handler),
    })
}

pub struct StepClock {
    values: Mutex<Vec<DateTime<FixedOffset>>>,
    calls: Mutex<usize>,
}

impl StepClock {
    pub fn new(values: &[&str]) -> Arc<Self> {
        let parsed = values.iter().map(|value| time(value)).collect();
        Arc::new(Self {
            values: Mutex::new(parsed),
            calls: Mutex::new(0),
        })
    }

    pub fn calls(&self) -> usize {
        *self.calls.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl SuccessClock for StepClock {
    fn now(&self) -> DateTime<FixedOffset> {
        *self.calls.lock().unwrap_or_else(PoisonError::into_inner) += 1;
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        if values.len() > 1 {
            values.remove(0)
        } else {
            *values.first().expect("a scripted clock value")
        }
    }
}

#[derive(Default)]
pub struct RecordingQueue {
    pub jobs: Mutex<Vec<StreamLoggingJob>>,
    pub rejecting: bool,
}

impl RecordingQueue {
    pub fn kinds(&self) -> Vec<StreamJobKind> {
        let jobs = self.jobs.lock().unwrap_or_else(PoisonError::into_inner);
        job_kinds(&jobs)
    }

    pub fn take_all(&self) -> Vec<StreamLoggingJob> {
        std::mem::take(&mut *self.jobs.lock().unwrap_or_else(PoisonError::into_inner))
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
pub struct RecordingDeferred {
    pub parked: Mutex<Vec<StreamLoggingJob>>,
}

impl RecordingDeferred {
    pub fn kinds(&self) -> Vec<StreamJobKind> {
        let jobs = self.parked.lock().unwrap_or_else(PoisonError::into_inner);
        job_kinds(&jobs)
    }

    pub fn take_all(&self) -> Vec<StreamLoggingJob> {
        std::mem::take(&mut *self.parked.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

fn job_kinds(jobs: &[StreamLoggingJob]) -> Vec<StreamJobKind> {
    let kinds = jobs.iter().map(|job| job.kind);
    kinds.collect()
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
pub struct RecordedRoute {
    pub url_route: String,
    pub request_body: Option<Json>,
    pub endpoint_type: EndpointType,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
    pub raw_bytes: Vec<Vec<u8>>,
    pub model: Option<String>,
}

#[derive(Default)]
pub struct RecordingRoute {
    pub inputs: Mutex<Vec<RecordedRoute>>,
}

impl RecordingRoute {
    pub fn calls(&self) -> Vec<RecordedRoute> {
        self.inputs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl StreamRoute for RecordingRoute {
    fn route(self: Arc<Self>, input: StreamLoggingInput) -> StreamWork {
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
        Box::pin(async {})
    }
}

#[derive(Default)]
pub struct RecordingCostInjection {
    pub calls: Mutex<Vec<(Vec<u8>, String)>>,
}

impl RecordingCostInjection {
    pub fn calls(&self) -> Vec<(Vec<u8>, String)> {
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
pub struct RecordingAnthropic {
    pub calls: Mutex<Vec<(Json, Vec<Vec<u8>>)>>,
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
pub struct RecordedFailure {
    pub class: String,
    pub message: String,
    pub traceback: String,
}

#[derive(Default)]
pub struct RecordingFailureDispatch {
    pub failures: Mutex<Vec<RecordedFailure>>,
}

impl RecordingFailureDispatch {
    pub fn failures(&self) -> Vec<RecordedFailure> {
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
            .push(RecordedFailure {
                class: failure.class.clone(),
                message: failure.message.clone(),
                traceback: failure.traceback.clone(),
            });
        Box::pin(async {})
    }
}

#[derive(Default)]
pub struct RecordingBinder {
    pub calls: Mutex<usize>,
}

impl RecordingBinder {
    pub fn calls(&self) -> usize {
        *self.calls.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl ReservationBinder for RecordingBinder {
    fn bind(&self, litellm_params: &mut IndexMap<String, Json>) {
        *self.calls.lock().unwrap_or_else(PoisonError::into_inner) += 1;
        stamp_callback_bound(litellm_params, true);
    }
}

#[derive(Default)]
pub struct RecordingTinyfish {
    pub poller: bool,
    pub run_id: Option<String>,
    pub scans: Mutex<Vec<Vec<u8>>>,
    pub started: Mutex<Vec<(String, DateTime<FixedOffset>)>>,
}

impl RecordingTinyfish {
    pub fn started(&self) -> Vec<(String, DateTime<FixedOffset>)> {
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
pub struct FakeProbe;

impl AnthropicFrameProbe for FakeProbe {
    fn is_message_stop(&self, _frames: &[u8]) -> bool {
        false
    }

    fn is_provider_error(&self, _frames: &[u8]) -> bool {
        false
    }
}

pub struct NoSuccessDispatch;

#[async_trait]
impl StreamSuccessDispatch for NoSuccessDispatch {
    async fn dispatch_success(&self, _call: StreamSuccessCall<'_>) {}
}

pub struct Harness {
    pub clock: Arc<StepClock>,
    pub queue: Arc<RecordingQueue>,
    pub deferred: Option<Arc<RecordingDeferred>>,
    pub binder: Arc<RecordingBinder>,
    pub route: Arc<RecordingRoute>,
    pub cost_injection: Arc<RecordingCostInjection>,
    pub anthropic: Arc<RecordingAnthropic>,
    pub failure_dispatch: Arc<RecordingFailureDispatch>,
    pub tinyfish: Arc<RecordingTinyfish>,
    pub handler: Arc<RecordingHandler>,
    pub providers: Arc<ProviderChunkHandlers>,
    pub probe: Arc<FakeProbe>,
    pub injection_override: Option<Arc<dyn StreamCostInjection>>,
}

impl Harness {
    pub fn new(values: &[&str]) -> Self {
        let handler = RecordingHandler::returning(Ok(HandlerOutcome::default()));
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
            providers: providers(&(Arc::clone(&handler) as Arc<dyn CollectedChunksHandler>)),
            handler,
            probe: Arc::new(FakeProbe),
            injection_override: None,
        }
    }

    pub fn with_handler_reply(mut self, reply: Result<HandlerOutcome, HandlerError>) -> Self {
        self.handler = RecordingHandler::returning(reply);
        self.providers = providers(&(Arc::clone(&self.handler) as Arc<dyn CollectedChunksHandler>));
        self
    }

    pub fn with_injection(mut self, injection: Arc<dyn StreamCostInjection>) -> Self {
        self.injection_override = Some(injection);
        self
    }

    pub fn with_deferred(mut self) -> Self {
        self.deferred = Some(Arc::new(RecordingDeferred::default()));
        self
    }

    pub fn with_rejecting_queue(mut self) -> Self {
        self.queue = Arc::new(RecordingQueue {
            jobs: Mutex::new(Vec::new()),
            rejecting: true,
        });
        self
    }

    pub fn with_tinyfish(mut self, poller: bool, run_id: Option<&str>) -> Self {
        self.tinyfish = Arc::new(RecordingTinyfish {
            poller,
            run_id: run_id.map(str::to_owned),
            scans: Mutex::new(Vec::new()),
            started: Mutex::new(Vec::new()),
        });
        self
    }

    pub fn deferred_handle(&self) -> &Arc<RecordingDeferred> {
        self.deferred
            .as_ref()
            .expect("the harness has deferred logging")
    }

    pub fn lifecycle(&self, cost_injection_enabled: bool) -> Arc<StreamLifecycle> {
        let cost_injection: Arc<dyn StreamCostInjection> = match &self.injection_override {
            Some(injection) => Arc::clone(injection),
            None => Arc::clone(&self.cost_injection),
        };
        Arc::new(StreamLifecycle {
            router: Arc::new(StreamLoggingRouter {
                providers: Arc::clone(&self.providers),
                frames: Arc::clone(&self.probe),
                tinyfish: Arc::clone(&self.tinyfish),
            }),
            passthrough: passthrough(),
            route: Arc::clone(&self.route),
            cost_injection,
            anthropic_partial_usage: Arc::clone(&self.anthropic),
            failure_dispatch: Arc::clone(&self.failure_dispatch),
            queue: Arc::clone(&self.queue),
            binder: Arc::clone(&self.binder),
            deferred: self
                .deferred
                .as_ref()
                .map(|deferred| deferred.clone() as Arc<dyn DeferredStreamLogging>),
            clock: Arc::clone(&self.clock),
            cost_injection_enabled,
        })
    }
}

pub fn processor_call(
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

pub fn ok_chunks(
    items: Vec<Vec<u8>>,
) -> impl Stream<Item = Result<Vec<u8>, StreamFailure>> + Send + 'static {
    stream::iter(items.into_iter().map(Ok).collect::<Vec<_>>())
}

pub fn failing_chunks(
    items: Vec<Vec<u8>>,
    failure: StreamFailure,
) -> impl Stream<Item = Result<Vec<u8>, StreamFailure>> + Send + 'static {
    let mut all: Vec<Result<Vec<u8>, StreamFailure>> = items.into_iter().map(Ok).collect();
    all.push(Err(failure));
    stream::iter(all)
}

pub fn failure() -> StreamFailure {
    StreamFailure {
        class: "ScriptedReadError".to_owned(),
        message: "the upstream read failed".to_owned(),
        traceback: "Traceback: scripted".to_owned(),
    }
}

#[derive(Clone, Default)]
pub struct Captured(pub Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub fn text(&self) -> String {
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

pub fn capture_logs() -> (Captured, DefaultGuard) {
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
