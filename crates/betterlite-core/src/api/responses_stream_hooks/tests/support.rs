use std::fmt::Debug;
use std::str::FromStr as _;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context as TaskContext, Poll, Waker};

use async_trait::async_trait;
use betterlite_error::error::ErrorClass;
use betterlite_error::error::details::{ResponseDetails, ResponseStatus};
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_logging::logging_success::types::SuccessRequest;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::ResponsesAPIStreamingResponse;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use tokio_util::task::TaskTracker;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context as LayerContext, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use crate::api::response_synthetic_events::stamping::{CostTarget, ResponsesCostCalculator};
use crate::api::responses_stream_hooks::ResponsesStreamHooks;
use crate::api::responses_stream_hooks::events::nested_response_mut;
use crate::api::responses_stream_hooks::ports::{
    CacheWrite, ChunkDeploymentHook, ChunkHookCall, DynamicCacheBackend, DynamicCacheObject,
    FailureDelivery, PortFailure, ResponseMetadataCall, ResponseMetadataTarget,
    ResponsesStreamCacheHandler, ResponsesStreamCacheTarget, ResponsesStreamFailureTarget,
    ResponsesStreamSuccessTarget, SuccessDeploymentHook,
};
use crate::api::responses_stream_primitives::state::{
    ResponsesStreamBaseState, ResponsesStreamHiddenParams,
};
use crate::routes::CallType;

pub(crate) const START: &str = "2026-01-01T00:00:00+00:00";
pub(crate) const END: &str = "2026-01-01T00:00:02+00:00";

const COMPLETED: &str = r#"{"type":"response.completed","sequence_number":3,"response":{"id":"resp_1","created_at":1700000000,"model":"gpt-4o","object":"response","status":"completed","output":[],"usage":{"input_tokens":10,"output_tokens":5,"total_tokens":15}}}"#;

pub(crate) fn timestamp(text: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(text).expect("the fixture time parses")
}

pub(crate) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds")
}

pub(crate) fn decode(text: &str) -> ResponsesAPIStreamingResponse {
    serde_json::from_str(text).expect("the fixture event decodes")
}

pub(crate) fn completed_event() -> ResponsesAPIStreamingResponse {
    decode(COMPLETED)
}

pub(crate) fn completed_with_id(id: &str) -> ResponsesAPIStreamingResponse {
    decode(&format!(
        r#"{{"type":"response.completed","sequence_number":3,"response":{{"id":"{id}","created_at":1700000000,"model":"gpt-4o","object":"response","status":"completed","output":[]}}}}"#
    ))
}

pub(crate) fn completed_with_hidden(hidden: Json) -> ResponsesAPIStreamingResponse {
    let mut event = completed_event();
    let response = nested_response_mut(&mut event).expect("a completed event nests a response");
    response.extra.insert("_hidden_params".to_owned(), hidden);
    event
}

pub(crate) fn response_failed_event(
    usage: Option<&str>,
    error: Option<&str>,
) -> ResponsesAPIStreamingResponse {
    let usage = usage.map_or(String::new(), |usage| format!(r#","usage":{usage}"#));
    let error = error.map_or(String::new(), |error| format!(r#","error":{error}"#));
    decode(&format!(
        r#"{{"type":"response.failed","sequence_number":2,"response":{{"id":"resp_1","created_at":1700000000,"model":"gpt-4o","object":"response","status":"failed","output":[]{error}{usage}}}}}"#
    ))
}

pub(crate) fn error_event(
    message: &str,
    error_type: &str,
    code: &str,
) -> ResponsesAPIStreamingResponse {
    decode(&format!(
        r#"{{"type":"error","sequence_number":5,"error":{{"type":"{error_type}","code":"{code}","message":"{message}"}}}}"#
    ))
}

pub(crate) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(crate) fn number(value: &str) -> Json {
    Json::Number(serde_json::Number::from_str(value).expect("the fixture number parses"))
}

pub(crate) fn json_object(pairs: &[(&str, Json)]) -> IndexMap<String, Json> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

pub(crate) fn json_value(pairs: &[(&str, Json)]) -> Json {
    Json::Object(json_object(pairs))
}

pub(crate) fn field<'json>(json: &'json Json, name: &str) -> Option<&'json Json> {
    match json {
        Json::Object(fields) => fields.get(name),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

pub(crate) fn path<'json>(json: &'json Json, names: &[&str]) -> Option<&'json Json> {
    names
        .iter()
        .try_fold(json, |current, name| field(current, name))
}

pub(crate) fn field_mut<'json>(json: &'json mut Json, name: &str) -> Option<&'json mut Json> {
    match json {
        Json::Object(fields) => fields.get_mut(name),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

pub(crate) fn path_mut<'json>(json: &'json mut Json, names: &[&str]) -> Option<&'json mut Json> {
    match names.split_first() {
        None => Some(json),
        Some((name, rest)) => field_mut(json, name).and_then(|value| path_mut(value, rest)),
    }
}

pub(crate) fn event_json(event: &ResponsesAPIStreamingResponse) -> Json {
    serde_json::to_value(event)
        .ok()
        .and_then(|value| serde_json::from_value::<Json>(value).ok())
        .unwrap_or(Json::Null)
}

pub(crate) fn base_state() -> ResponsesStreamBaseState {
    ResponsesStreamBaseState {
        model: "gpt-4o".to_owned(),
        custom_llm_provider: Some("openai".to_owned()),
        litellm_metadata: None,
        request_data: IndexMap::new(),
        call_type: Some("responses".to_owned()),
        finished: false,
        failure_handled: false,
        yielded_first_chunk: false,
        output_started: false,
        completed_response: None,
        completed_response_cached: false,
        completed_response_logged: false,
        completed_response_cache_hit: None,
        persist_before_logging: true,
        generated_content: String::new(),
        generated_tool_arguments: String::new(),
        start_time: timestamp(START),
        created_timestamp: 1_700_000_000.0,
        max_streaming_duration_seconds: None,
        hidden_params: ResponsesStreamHiddenParams {
            model_id: None,
            api_base: None,
            custom_llm_provider: Some("openai".to_owned()),
            additional_headers: IndexMap::new(),
        },
        raw_response_headers: IndexMap::new(),
        pending_tasks: TaskTracker::new(),
    }
}

pub(crate) struct FixedClock(pub DateTime<FixedOffset>);

impl SuccessClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        self.0
    }
}

pub(crate) fn hooks(redactor: &Redactor) -> ResponsesStreamHooks<'_> {
    ResponsesStreamHooks {
        cache: None,
        cache_handler: None,
        success: None,
        failure: None,
        metadata: None,
        success_hook: None,
        chunk_hooks: Vec::new(),
        clock: Arc::new(FixedClock(timestamp(END))),
        redactor,
    }
}

pub(crate) struct DualToken;

impl DynamicCacheBackend for DualToken {}

pub(crate) fn dual_token() -> DynamicCacheObject {
    DynamicCacheObject::shared(Arc::new(DualToken))
}

pub(crate) struct CacheHandler {
    kwargs: Option<IndexMap<String, Json>>,
    preset: Option<String>,
    original: Option<String>,
    dual: Option<DynamicCacheObject>,
    store: bool,
    seen: Mutex<Vec<IndexMap<String, Json>>>,
}

impl CacheHandler {
    pub(crate) fn new(kwargs: IndexMap<String, Json>) -> Self {
        Self {
            kwargs: Some(kwargs),
            preset: None,
            original: Some("aresponses".to_owned()),
            dual: None,
            store: true,
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn without_request_kwargs() -> Self {
        Self {
            kwargs: None,
            ..Self::new(IndexMap::new())
        }
    }

    pub(crate) fn with_preset(mut self, preset: &str) -> Self {
        self.preset = Some(preset.to_owned());
        self
    }

    pub(crate) fn with_dual(mut self, dual: DynamicCacheObject) -> Self {
        self.dual = Some(dual);
        self
    }

    pub(crate) fn with_store(mut self, store: bool) -> Self {
        self.store = store;
        self
    }

    pub(crate) fn seen(&self) -> Vec<IndexMap<String, Json>> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ResponsesStreamCacheHandler for CacheHandler {
    fn request_kwargs(&self) -> Option<&IndexMap<String, Json>> {
        self.kwargs.as_ref()
    }

    fn preset_cache_key(&self) -> Option<&str> {
        self.preset.as_deref()
    }

    fn original_function(&self) -> Option<&str> {
        self.original.as_deref()
    }

    fn dual_cache(&self) -> Option<DynamicCacheObject> {
        self.dual.clone()
    }

    fn should_store_result_in_cache(
        &self,
        _original_function: Option<&str>,
        kwargs: &IndexMap<String, Json>,
    ) -> Result<bool, PortFailure> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(kwargs.clone());
        Ok(self.store)
    }
}

#[derive(Clone)]
pub(crate) struct RecordedWrite {
    pub payload: String,
    pub request_kwargs: IndexMap<String, Json>,
    pub dynamic: Option<DynamicCacheObject>,
    pub asynchronous: bool,
}

pub(crate) struct Cache {
    writes: Mutex<Vec<RecordedWrite>>,
    failure: Option<String>,
}

impl Cache {
    pub(crate) fn succeeding() -> Arc<Self> {
        Arc::new(Self {
            writes: Mutex::new(Vec::new()),
            failure: None,
        })
    }

    pub(crate) fn failing(message: &str) -> Arc<Self> {
        Arc::new(Self {
            writes: Mutex::new(Vec::new()),
            failure: Some(message.to_owned()),
        })
    }

    pub(crate) fn writes(&self) -> Vec<RecordedWrite> {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, write: CacheWrite, asynchronous: bool) -> Result<(), PortFailure> {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedWrite {
                payload: write.payload,
                request_kwargs: write.request_kwargs,
                dynamic: write.dynamic_cache,
                asynchronous,
            });
        match &self.failure {
            Some(message) => Err(PortFailure {
                message: message.clone(),
            }),
            None => Ok(()),
        }
    }
}

#[async_trait]
impl ResponsesStreamCacheTarget for Cache {
    fn add_cache(&self, write: CacheWrite) -> Result<(), PortFailure> {
        self.record(write, false)
    }

    async fn async_add_cache(&self, write: CacheWrite) -> Result<(), PortFailure> {
        self.record(write, true)
    }
}

pub(crate) struct Success {
    dispatches: Mutex<Vec<SuccessRequest>>,
    async_calls: Mutex<Vec<SuccessRequest>>,
    sync_calls: Mutex<Vec<SuccessRequest>>,
    order: Mutex<Vec<&'static str>>,
    deferred: bool,
}

impl Success {
    pub(crate) fn recording() -> Arc<Self> {
        Arc::new(Self {
            dispatches: Mutex::new(Vec::new()),
            async_calls: Mutex::new(Vec::new()),
            sync_calls: Mutex::new(Vec::new()),
            order: Mutex::new(Vec::new()),
            deferred: false,
        })
    }

    pub(crate) fn deferred() -> Arc<Self> {
        Arc::new(Self {
            dispatches: Mutex::new(Vec::new()),
            async_calls: Mutex::new(Vec::new()),
            sync_calls: Mutex::new(Vec::new()),
            order: Mutex::new(Vec::new()),
            deferred: true,
        })
    }

    pub(crate) fn dispatches(&self) -> Vec<SuccessRequest> {
        self.dispatches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn async_calls(&self) -> Vec<SuccessRequest> {
        self.async_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn sync_calls(&self) -> Vec<SuccessRequest> {
        self.sync_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn order(&self) -> Vec<&'static str> {
        self.order
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl ResponsesStreamSuccessTarget for Success {
    async fn dispatch_success_handlers(&self, request: SuccessRequest) {
        self.order
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("dispatch");
        self.dispatches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
    }

    async fn async_success_handler(&self, request: &SuccessRequest) {
        self.order
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("async");
        self.async_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
    }

    fn success_handler(&self, request: &SuccessRequest) {
        self.order
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("sync");
        self.sync_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
    }

    fn deferred_stream_complete_armed(&self) -> bool {
        self.deferred
    }
}

pub(crate) struct FailureRecorder {
    async_calls: Mutex<Vec<FailureDelivery>>,
    sync_calls: Mutex<Vec<FailureDelivery>>,
    order: Mutex<Vec<&'static str>>,
    async_fails: bool,
    submit_fails: bool,
}

impl FailureRecorder {
    pub(crate) fn recording() -> Arc<Self> {
        Arc::new(Self::empty())
    }

    pub(crate) fn failing_async() -> Arc<Self> {
        Arc::new(Self {
            async_fails: true,
            ..Self::empty()
        })
    }

    pub(crate) fn failing_submit() -> Arc<Self> {
        Arc::new(Self {
            submit_fails: true,
            ..Self::empty()
        })
    }

    fn empty() -> Self {
        Self {
            async_calls: Mutex::new(Vec::new()),
            sync_calls: Mutex::new(Vec::new()),
            order: Mutex::new(Vec::new()),
            async_fails: false,
            submit_fails: false,
        }
    }

    pub(crate) fn async_calls(&self) -> Vec<FailureDelivery> {
        self.async_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn sync_calls(&self) -> Vec<FailureDelivery> {
        self.sync_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn order(&self) -> Vec<&'static str> {
        self.order
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl ResponsesStreamFailureTarget for FailureRecorder {
    async fn async_failure_handler(&self, delivery: &FailureDelivery) -> Result<(), PortFailure> {
        self.order
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("async");
        self.async_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(delivery.clone());
        if self.async_fails {
            return Err(PortFailure {
                message: "async failed".to_owned(),
            });
        }
        Ok(())
    }

    fn failure_handler(&self, delivery: &FailureDelivery) -> Result<(), PortFailure> {
        self.order
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("sync");
        self.sync_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(delivery.clone());
        if self.submit_fails {
            return Err(PortFailure {
                message: "submit failed".to_owned(),
            });
        }
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct RecordedMetadata {
    pub result: Json,
    pub model: String,
    pub kwargs: IndexMap<String, Json>,
    pub include_overhead: bool,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
}

pub(crate) struct Metadata {
    calls: Mutex<Vec<RecordedMetadata>>,
    fails: bool,
}

impl Metadata {
    pub(crate) fn recording() -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            fails: false,
        })
    }

    pub(crate) fn failing() -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            fails: true,
        })
    }

    pub(crate) fn calls(&self) -> Vec<RecordedMetadata> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ResponseMetadataTarget for Metadata {
    fn update_response_metadata(&self, call: ResponseMetadataCall<'_>) -> Result<(), PortFailure> {
        let result = serde_json::to_value(call.result)
            .ok()
            .and_then(|value| serde_json::from_value::<Json>(value).ok())
            .unwrap_or(Json::Null);
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedMetadata {
                result,
                model: call.model.to_owned(),
                kwargs: call.kwargs.clone(),
                include_overhead: call.include_overhead,
                start_time: call.start_time,
                end_time: call.end_time,
            });
        if self.fails {
            return Err(PortFailure {
                message: "metadata failed".to_owned(),
            });
        }
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct RecordedHookCall {
    pub request_data: IndexMap<String, Json>,
    pub response: Json,
    pub call_type: String,
}

pub(crate) struct SuccessHook {
    calls: Mutex<Vec<RecordedHookCall>>,
    replacement: Option<Json>,
    fails: bool,
}

impl SuccessHook {
    pub(crate) fn recording() -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            replacement: None,
            fails: false,
        })
    }

    pub(crate) fn replacing(replacement: Json) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            replacement: Some(replacement),
            fails: false,
        })
    }

    pub(crate) fn failing() -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            replacement: None,
            fails: true,
        })
    }

    pub(crate) fn calls(&self) -> Vec<RecordedHookCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl SuccessDeploymentHook for SuccessHook {
    async fn success_deployment_hook(
        &self,
        request_data: &IndexMap<String, Json>,
        response: &Json,
        call_type: &str,
    ) -> Result<Json, PortFailure> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedHookCall {
                request_data: request_data.clone(),
                response: response.clone(),
                call_type: call_type.to_owned(),
            });
        if self.fails {
            return Err(PortFailure {
                message: "the success hook failed".to_owned(),
            });
        }
        Ok(self.replacement.clone().unwrap_or_else(|| response.clone()))
    }
}

pub(crate) enum ChunkAction {
    Keep,
    Replace(Box<ResponsesAPIStreamingResponse>),
    Fail,
}

pub(crate) struct ChunkHook {
    name: &'static str,
    action: ChunkAction,
    calls: Mutex<Vec<&'static str>>,
    call_types: Mutex<Vec<Option<CallType>>>,
    requests: Mutex<Vec<IndexMap<String, Json>>>,
}

impl ChunkHook {
    pub(crate) fn new(name: &'static str, action: ChunkAction) -> Arc<Self> {
        Arc::new(Self {
            name,
            action,
            calls: Mutex::new(Vec::new()),
            call_types: Mutex::new(Vec::new()),
            requests: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn calls(&self) -> Vec<&'static str> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn call_types(&self) -> Vec<Option<CallType>> {
        self.call_types
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn requests(&self) -> Vec<IndexMap<String, Json>> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl ChunkDeploymentHook for ChunkHook {
    async fn post_streaming_deployment_hook(
        &self,
        call: ChunkHookCall<'_>,
    ) -> Result<Option<ResponsesAPIStreamingResponse>, PortFailure> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(self.name);
        self.call_types
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call.call_type);
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call.request_data.clone());
        match &self.action {
            ChunkAction::Keep => Ok(None),
            ChunkAction::Replace(replacement) => Ok(Some(replacement.as_ref().clone())),
            ChunkAction::Fail => Err(PortFailure {
                message: "chunk hook failed".to_owned(),
            }),
        }
    }
}

pub(crate) struct Calculator {
    pub cost: Option<Decimal>,
    pub fails: bool,
}

impl Calculator {
    pub(crate) fn charging(cost: Decimal) -> Self {
        Self {
            cost: Some(cost),
            fails: false,
        }
    }

    pub(crate) fn missing() -> Self {
        Self {
            cost: None,
            fails: false,
        }
    }

    pub(crate) fn failing() -> Self {
        Self {
            cost: None,
            fails: true,
        }
    }
}

impl ResponsesCostCalculator for Calculator {
    type Error = std::io::Error;

    fn response_cost(&self, _target: &CostTarget<'_>) -> Result<Option<Decimal>, Self::Error> {
        if self.fails {
            return Err(std::io::Error::other("the cost lookup failed"));
        }
        Ok(self.cost)
    }
}

pub(crate) fn failure_class() -> ErrorClass {
    ErrorClass::BadRequest(Box::new(ResponseDetails {
        message: "the provider rejected the stream".to_owned(),
        model: Some("gpt-4o".to_owned()),
        provider: Some("openai".to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
        status: ResponseStatus(400),
    }))
}

// polls a future once with a noop waker for the branches that only run without a tokio runtime
pub(crate) fn block_on_local<F: std::future::Future>(future: F) -> F::Output {
    let waker = Waker::noop();
    let mut context = TaskContext::from_waker(waker);
    let mut future = std::pin::pin!(future);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("the probe future must not park"),
    }
}

#[derive(Clone, Default)]
pub(crate) struct CapturedLogs(Arc<Mutex<Vec<String>>>);

impl CapturedLogs {
    pub(crate) fn messages(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for CapturedLogs {
    fn on_event(&self, event: &Event<'_>, _context: LayerContext<'_, Inner>) {
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.0);
    }
}

pub(crate) fn capture<T>(call: impl FnOnce() -> T) -> (T, CapturedLogs) {
    let logs = CapturedLogs::default();
    let subscriber = registry().with(logs.clone());
    let value = with_default(subscriber, call);
    (value, logs)
}
