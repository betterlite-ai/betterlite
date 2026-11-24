use std::mem::take;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_logging::logging_state::LoggingInit;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_logging_state::logger::PassThroughEndpointLogging;
use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_logging::passthrough_stream_lifecycle::chunk_processor::{
    ChunkProcessorCall, chunk_processor,
};
use betterlite_logging::passthrough_stream_lifecycle::cost_injection::StreamCostInjection;
use betterlite_logging::passthrough_stream_lifecycle::failure::{
    StreamFailure, StreamFailureDispatch,
};
use betterlite_logging::passthrough_stream_lifecycle::lifecycle::StreamLifecycle;
use betterlite_logging::passthrough_stream_lifecycle::partial_usage::AnthropicPartialUsage;
use betterlite_logging::passthrough_stream_lifecycle::ports::{
    DeferredStreamLogging, StreamLoggingJob, StreamLoggingQueue, StreamRoute, StreamWork,
};
use betterlite_logging::passthrough_stream_logging::build::StreamLoggingInput;
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamLoggingState, StreamSuccessCall, StreamSuccessDispatch,
};
use betterlite_logging::passthrough_stream_logging::router::{
    AnthropicFrameProbe, StreamLoggingRouter,
};
use betterlite_logging::passthrough_stream_logging::tinyfish::{
    TinyfishCollectedChunks, TinyfishStreamBilling,
};
use betterlite_logging::passthrough_stream_logging::types::{
    CollectedChunksCall, CollectedChunksHandler, EndpointType, HandlerError, HandlerOutcome,
    ProviderChunkHandlers,
};
use betterlite_logging::reservation_binding::{ReservationBinder, stamp_callback_bound};
use betterlite_logging::worker::Admission;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use futures_util::stream;
use futures_util::{Stream, StreamExt};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| invalid(&error.to_string()))?;
        runtime.block_on(execute(input))
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    endpoint_type: EndpointName,
    chunks: Vec<Vec<u8>>,
    failure: Option<FailureSpec>,
    disconnect_after: Option<usize>,
    status_code: u16,
    cost_injection: bool,
    request_body: Option<Value>,
    url_route: String,
    deferred: bool,
    tinyfish_poller: bool,
    run_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FailureSpec {
    message: String,
    traceback: String,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EndpointName {
    #[serde(rename = "vertex-ai")]
    VertexAi,
    Gemini,
    Anthropic,
    #[serde(rename = "openai")]
    OpenAi,
    Tinyfish,
    Generic,
    Decisions,
}

impl From<EndpointName> for EndpointType {
    fn from(name: EndpointName) -> Self {
        match name {
            EndpointName::VertexAi => Self::VertexAi,
            EndpointName::Gemini => Self::Gemini,
            EndpointName::Anthropic => Self::Anthropic,
            EndpointName::OpenAi => Self::OpenAi,
            EndpointName::Tinyfish => Self::Tinyfish,
            EndpointName::Generic => Self::Generic,
            EndpointName::Decisions => Self::Decisions,
        }
    }
}

async fn execute(input: &Input) -> HelperResult {
    let logging_obj = logging_object()?;
    let handler: Arc<dyn CollectedChunksHandler> = Arc::new(Handler);
    let providers = Arc::new(ProviderChunkHandlers {
        anthropic: Arc::clone(&handler),
        vertex: Arc::clone(&handler),
        gemini: Arc::clone(&handler),
        openai: handler,
    });
    let recorder = Arc::new(Recorder::new(input.tinyfish_poller, input.run_id.clone()));
    let shared = Arc::new(PassThrough);
    let router = Arc::new(StreamLoggingRouter {
        providers,
        frames: Arc::clone(&shared) as Arc<dyn AnthropicFrameProbe>,
        tinyfish: Arc::clone(&recorder) as Arc<dyn TinyfishStreamBilling>,
    });
    let lifecycle = Arc::new(StreamLifecycle {
        router,
        passthrough: Arc::new(PassThroughEndpointLogging {
            injected_log_dispatch: None,
        }),
        route: Arc::clone(&recorder) as Arc<dyn StreamRoute>,
        cost_injection: Arc::clone(&recorder) as Arc<dyn StreamCostInjection>,
        anthropic_partial_usage: shared as Arc<dyn AnthropicPartialUsage>,
        failure_dispatch: Arc::clone(&recorder) as Arc<dyn StreamFailureDispatch>,
        queue: Arc::clone(&recorder) as Arc<dyn StreamLoggingQueue>,
        binder: Arc::clone(&recorder) as Arc<dyn ReservationBinder>,
        deferred: if input.deferred {
            Some(Arc::clone(&recorder) as Arc<dyn DeferredStreamLogging>)
        } else {
            None
        },
        clock: Arc::new(FixedClock(fixed_time()?)),
        cost_injection_enabled: input.cost_injection,
    });
    let call = ChunkProcessorCall {
        lifecycle: Arc::clone(&lifecycle),
        logging_obj: Arc::clone(&logging_obj),
        endpoint_type: EndpointType::from(input.endpoint_type),
        start_time: fixed_start()?,
        url_route: input.url_route.clone(),
        request_body: request_body(input.request_body.as_ref())?,
        status_code: input.status_code,
    };
    let mut items: Vec<Result<Vec<u8>, StreamFailure>> =
        input.chunks.iter().cloned().map(Ok).collect();
    if let Some(spec) = &input.failure {
        items.push(Err(StreamFailure {
            class: "ScriptedReadError".to_owned(),
            message: spec.message.clone(),
            traceback: spec.traceback.clone(),
        }));
    }
    let mut chunks = Box::pin(chunk_processor(call, stream::iter(items)));
    let (yielded, failure, disconnected) = drive(input.disconnect_after, &mut chunks).await;
    drop(chunks);
    let queued = take(&mut *lock(&recorder.queued));
    let queued_jobs = queued.len();
    for job in queued {
        job.work.await;
    }
    let parked = take(&mut *lock(&recorder.parked));
    let deferred_jobs = parked.len();
    for job in parked {
        job.work.await;
    }
    Ok(answer(
        &logging_obj,
        &recorder,
        Driven {
            yielded,
            failure,
            disconnected,
            queued_jobs,
            deferred_jobs,
        },
    ))
}

struct Driven {
    yielded: Vec<Vec<u8>>,
    failure: Option<StreamFailure>,
    disconnected: bool,
    queued_jobs: usize,
    deferred_jobs: usize,
}

async fn drive<S>(
    disconnect_after: Option<usize>,
    chunks: &mut Pin<Box<S>>,
) -> (Vec<Vec<u8>>, Option<StreamFailure>, bool)
where
    S: Stream<Item = Result<Vec<u8>, StreamFailure>> + Send,
{
    let mut yielded = Vec::new();
    let mut failure = None;
    let mut disconnected = false;
    match disconnect_after {
        Some(limit) => {
            while yielded.len() < limit {
                match chunks.next().await {
                    Some(Ok(chunk)) => yielded.push(chunk),
                    Some(Err(error)) => {
                        failure = Some(error);
                        break;
                    }
                    None => break,
                }
            }
            disconnected = failure.is_none() && yielded.len() == limit;
        }
        None => {
            while let Some(item) = chunks.next().await {
                match item {
                    Ok(chunk) => yielded.push(chunk),
                    Err(error) => {
                        failure = Some(error);
                        break;
                    }
                }
            }
        }
    }
    (yielded, failure, disconnected)
}

fn answer(logging_obj: &StreamLoggingObject, recorder: &Recorder, driven: Driven) -> Value {
    let details = logging_obj
        .read(&mut |state| state.model_call_details.clone())
        .unwrap_or_default();
    let params = logging_obj
        .read(&mut |state| state.litellm_params.clone())
        .unwrap_or_default();
    let routes = lock(&recorder.routes);
    let route_calls = routes.len();
    let route_bytes = routes.first().cloned();
    drop(routes);
    let completion = logging_obj
        .read(&mut |state| state.completion_start_time.map(|value| value.to_rfc3339()))
        .flatten();
    let outcome = if driven.failure.is_some() {
        "failed"
    } else if driven.disconnected {
        "disconnected"
    } else {
        "completed"
    };
    json!({
        "yielded": driven.yielded,
        "outcome": outcome,
        "error": driven.failure.as_ref().map(ToString::to_string),
        "completion_start_time": completion,
        "route_calls": route_calls,
        "route_raw_bytes": route_bytes,
        "queued_jobs": driven.queued_jobs,
        "deferred_jobs": driven.deferred_jobs,
        "failures": lock(&recorder.failures)
            .iter()
            .map(|failure| json!({
                "class": failure.class,
                "message": failure.message,
                "traceback_tail": failure.traceback.lines().last().unwrap_or_default(),
            }))
            .collect::<Vec<_>>(),
        "partial_usage": partial_usage(&details),
        "tinyfish_started": lock(&recorder.started)
            .iter()
            .map(|(run_id, at)| json!([run_id, at.to_rfc3339()]))
            .collect::<Vec<_>>(),
        "tinyfish_scans": *lock(&recorder.scans),
        "callback_bound": callback_bound(&params),
    })
}

struct Handler;

impl CollectedChunksHandler for Handler {
    fn handle(&self, _call: &CollectedChunksCall<'_>) -> Result<HandlerOutcome, HandlerError> {
        let body: Json = serde_json::from_value(json!({
            "id": "chatcmpl-1",
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15},
        }))
        .map_err(HandlerError::leaf)?;
        let mut kwargs = PassThroughLoggingKwargs::default();
        if let Some(number) = serde_json::Number::from_f64(0.25) {
            kwargs
                .entries
                .insert("response_cost".to_owned(), Json::Number(number));
        }
        Ok(HandlerOutcome {
            result: Some(PassThroughResponse::ModelResponse(body)),
            kwargs,
        })
    }
}

struct PassThrough;

impl AnthropicFrameProbe for PassThrough {
    fn is_message_stop(&self, _frames: &[u8]) -> bool {
        false
    }

    fn is_provider_error(&self, _frames: &[u8]) -> bool {
        false
    }
}

impl AnthropicPartialUsage for PassThrough {
    fn record_partial_usage(
        &self,
        _logging_obj: &StreamLoggingObject,
        _request_body: &Json,
        _all_chunks: &[Vec<u8>],
    ) {
    }
}

#[async_trait]
impl StreamSuccessDispatch for PassThrough {
    async fn dispatch_success(&self, _call: StreamSuccessCall<'_>) {}
}

struct FixedClock(DateTime<FixedOffset>);

impl SuccessClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        self.0
    }
}

struct Recorder {
    poller: bool,
    run_id: Option<String>,
    routes: Mutex<Vec<Vec<Vec<u8>>>>,
    queued: Mutex<Vec<StreamLoggingJob>>,
    parked: Mutex<Vec<StreamLoggingJob>>,
    failures: Mutex<Vec<StreamFailure>>,
    scans: Mutex<usize>,
    started: Mutex<Vec<(String, DateTime<FixedOffset>)>>,
}

impl Recorder {
    fn new(poller: bool, run_id: Option<String>) -> Self {
        Self {
            poller,
            run_id,
            routes: Mutex::new(Vec::new()),
            queued: Mutex::new(Vec::new()),
            parked: Mutex::new(Vec::new()),
            failures: Mutex::new(Vec::new()),
            scans: Mutex::new(0),
            started: Mutex::new(Vec::new()),
        }
    }
}

impl StreamRoute for Recorder {
    fn route(self: Arc<Self>, input: StreamLoggingInput) -> StreamWork {
        lock(&self.routes).push(input.raw_bytes);
        Box::pin(async {})
    }
}

impl StreamLoggingQueue for Recorder {
    fn submit(&self, job: StreamLoggingJob) -> Admission {
        lock(&self.queued).push(job);
        Admission::Accepted
    }
}

impl DeferredStreamLogging for Recorder {
    fn park(&self, job: StreamLoggingJob) {
        lock(&self.parked).push(job);
    }
}

impl StreamCostInjection for Recorder {
    fn inject(
        &self,
        frames: &[u8],
        _model_name: &str,
        _logging_obj: &StreamLoggingObject,
    ) -> Vec<u8> {
        let mut output = b"injected:".to_vec();
        output.extend_from_slice(frames);
        output
    }
}

impl ReservationBinder for Recorder {
    fn bind(&self, litellm_params: &mut IndexMap<String, Json>) {
        stamp_callback_bound(litellm_params, true);
    }
}

impl StreamFailureDispatch for Recorder {
    fn dispatch_failure(
        &self,
        _logging_obj: Arc<StreamLoggingObject>,
        failure: &StreamFailure,
    ) -> StreamWork {
        lock(&self.failures).push(failure.clone());
        Box::pin(async {})
    }
}

#[async_trait]
impl TinyfishStreamBilling for Recorder {
    // LiteLLM marks the poller spawned when billing starts, so the flag alone is not enough
    fn sse_poller_spawned(&self, _logging_obj: &StreamLoggingObject) -> bool {
        self.poller && !lock(&self.started).is_empty()
    }

    fn run_id_from_sse_frames(&self, _frames: &[u8]) -> Option<String> {
        *lock(&self.scans) += 1;
        self.run_id.clone()
    }

    fn start_sse_run_billing(
        &self,
        run_id: &str,
        _logging_obj: &StreamLoggingObject,
        start_time: DateTime<FixedOffset>,
    ) {
        lock(&self.started).push((run_id.to_owned(), start_time));
    }

    // ast-grep-ignore: no-long-names
    async fn handle_logging_tinyfish_collected_chunks(
        &self,
        _call: &TinyfishCollectedChunks<'_>,
    ) -> Result<HandlerOutcome, HandlerError> {
        Ok(HandlerOutcome::default())
    }
}

fn logging_object() -> Result<Arc<StreamLoggingObject>, Box<HelperError>> {
    let init = LoggingInit {
        model: "gpt-4o".to_owned(),
        call_type: "pass_through_endpoint".to_owned(),
        start_time: fixed_start()?,
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
    let mut correlation = CorrelationContext::default();
    let setup = CorrelationSetup {
        context: &mut correlation,
        settings: CorrelationSettings::default(),
    };
    let redactor = Redactor::new(16).map_err(|error| invalid(&error.to_string()))?;
    let mut logging = LoggingState::new(
        init,
        setup,
        &redactor,
        || "trace-1".to_owned(),
        || Duration::ZERO,
    )
    .map_err(|error| invalid(&error.to_string()))?;
    logging.litellm_params = reservation_params();
    Ok(Arc::new(StreamLoggingObject {
        slot: Mutex::new(Some(StreamLoggingState {
            logging,
            correlation: CorrelationContext::default(),
        })),
        dispatch: Arc::new(PassThrough),
    }))
}

fn reservation_params() -> IndexMap<String, Json> {
    let mut reservation = IndexMap::new();
    reservation.insert("id".to_owned(), Json::Text("reservation-1".to_owned()));
    let mut metadata = IndexMap::new();
    metadata.insert(
        "user_api_key_budget_reservation".to_owned(),
        Json::Object(reservation),
    );
    let mut params = IndexMap::new();
    params.insert("metadata".to_owned(), Json::Object(metadata));
    params
}

fn partial_usage(details: &IndexMap<String, Json>) -> Option<Value> {
    let Json::Object(usage) = details.get("combined_usage_object")? else {
        return None;
    };
    let Json::Number(cost) = details.get("response_cost")? else {
        return None;
    };
    let prompt = tokens(usage, "prompt_tokens")?;
    let completion = tokens(usage, "completion_tokens")?;
    let total = tokens(usage, "total_tokens")?;
    Some(json!({
        "prompt_tokens": prompt,
        "completion_tokens": completion,
        "total_tokens": total,
        "response_cost": cost.to_string(),
    }))
}

fn tokens(usage: &IndexMap<String, Json>, key: &str) -> Option<i64> {
    let Some(Json::Number(number)) = usage.get(key) else {
        return None;
    };
    number.as_i64()
}

fn callback_bound(params: &IndexMap<String, Json>) -> Option<bool> {
    let Json::Object(metadata) = params.get("metadata")? else {
        return None;
    };
    let Json::Object(reservation) = metadata.get("user_api_key_budget_reservation")? else {
        return None;
    };
    let Json::Bool(flag) = reservation.get("callback_bound")? else {
        return None;
    };
    Some(*flag)
}

fn request_body(value: Option<&Value>) -> Result<Option<Json>, Box<HelperError>> {
    value
        .map(|value| {
            serde_json::from_value(value.clone()).map_err(|error| invalid(&error.to_string()))
        })
        .transpose()
}

fn fixed_start() -> Result<DateTime<FixedOffset>, Box<HelperError>> {
    time_of("2024-01-01T00:00:00+00:00")
}

fn fixed_time() -> Result<DateTime<FixedOffset>, Box<HelperError>> {
    time_of("2024-01-02T00:00:00+00:00")
}

fn time_of(text: &str) -> Result<DateTime<FixedOffset>, Box<HelperError>> {
    DateTime::parse_from_rfc3339(text).map_err(|error| invalid(&error.to_string()))
}

fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    value.lock().unwrap_or_else(PoisonError::into_inner)
}

fn invalid(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
