#![cfg(test)]

use std::io::Error as IoError;
use std::str::FromStr as _;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
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
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::event_types::Extra;
use betterlite_openai::api::responses_event_records::records::{
    GenericEvent, ResponsesAPIStreamingResponse,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use tokio_util::task::TaskTracker;

use super::{
    ProcessEventError, ProjectQuotaCallback, ResponsesEventProcessor, ResponsesEventTransform,
    ResponsesStreamBaseState, ResponsesStreamHiddenParams,
};
use crate::api::api_boundaries::ApiLimits;
use crate::api::response_id_codecs::container::decode_container_id;
use crate::api::response_id_codecs::encrypted::{unwrap_encrypted_content, wrap_encrypted_content};
use crate::api::response_id_codecs::response::{build_response_id, decode_response_id};
use crate::api::response_synthetic_events::estimate::{
    EstimateRequest, TokenCountRequest, UsageTokenization,
};
use crate::api::response_synthetic_events::stamping::{CostTarget, ResponsesCostCalculator};
use crate::api::responses_stream_hooks::ResponsesStreamHooks;
use crate::api::responses_stream_hooks::failure::await_pending_logging;
use crate::api::responses_stream_hooks::ports::{
    CacheWrite, Dispatch, FailureDelivery, PortFailure, ResponsesStreamCacheHandler,
    ResponsesStreamCacheTarget, ResponsesStreamFailureTarget, ResponsesStreamSuccessTarget,
};
use crate::api::responses_stream_hooks::state::ResponsesStreamHookState;

const START: &str = "2026-01-01T00:00:00+00:00";
const CLOCK: &str = "2026-01-01T00:00:02+00:00";

const TEXT_DELTA: &str = r#"{"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"content_index":0,"delta":"hi"}"#;
const ARGS_DELTA: &str = r#"{"type":"response.function_call_arguments.delta","item_id":"call_1","output_index":0,"delta":"{}"}"#;
const CUSTOM_DELTA: &str = r#"{"type":"response.custom_tool_call_input.delta","item_id":"ct_1","output_index":0,"delta":"X"}"#;
const MCP_DELTA: &str = r#"{"type":"response.mcp_call_arguments.delta","item_id":"mcp_1","output_index":0,"sequence_number":2,"delta":"Y"}"#;
const REFUSAL_DELTA: &str = r#"{"type":"response.refusal.delta","item_id":"msg_1","output_index":0,"content_index":0,"delta":"no"}"#;
const GENERIC_TEXT_DELTA: &str =
    r#"{"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"delta":"Z"}"#;
const NON_STRING_DELTA: &str = r#"{"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"content_index":0,"delta":7}"#;

const CREATED: &str = r#"{"type":"response.created","sequence_number":0,"response":{"id":"resp_1","created_at":1,"object":"response","output":[{"type":"message","id":"msg_1","container_id":"c"}]}}"#;
const IN_PROGRESS: &str = r#"{"type":"response.in_progress","sequence_number":1,"response":{"id":"resp_1","created_at":1,"object":"response","status":"in_progress","output":[{"type":"message","id":"msg_1","container_id":"c"}]}}"#;
const COMPLETED: &str = r#"{"type":"response.completed","sequence_number":3,"response":{"id":"resp_1","created_at":1700000000,"object":"response","status":"completed","output":[]}}"#;
const COMPLETED_USAGE: &str = r#"{"type":"response.completed","sequence_number":3,"response":{"id":"resp_1","created_at":1700000000,"object":"response","status":"completed","output":[],"usage":{"input_tokens":7,"output_tokens":9,"total_tokens":16}}}"#;
const COMPLETED_COST: &str = r#"{"type":"response.completed","sequence_number":3,"response":{"id":"resp_2","created_at":1700000000,"object":"response","status":"completed","output":[],"usage":{"input_tokens":7,"output_tokens":9,"total_tokens":16,"cost":0.25}}}"#;
const INCOMPLETE: &str = r#"{"type":"response.incomplete","sequence_number":3,"response":{"id":"resp_1","created_at":1700000000,"object":"response","status":"incomplete","output":[]}}"#;
const FAILED_USAGE: &str = r#"{"type":"response.failed","sequence_number":3,"response":{"id":"resp_1","created_at":1700000000,"object":"response","status":"failed","error":{"code":"server_error","message":"boom"},"output":[],"usage":{"input_tokens":7,"output_tokens":9,"total_tokens":16}}}"#;
const FAILED_BARE: &str = r#"{"type":"response.failed","sequence_number":3,"response":{"id":"resp_1","created_at":1700000000,"object":"response","status":"failed","output":[]}}"#;
const GENERIC_COMPLETED: &str = r#"{"type":"response.completed","response":{"id":"resp_1","created_at":1700000000,"object":"response","usage":{"input_tokens":7,"output_tokens":9,"total_tokens":16}}}"#;
const GENERIC_INCOMPLETE: &str = r#"{"type":"response.incomplete","response":{"id":"resp_1","created_at":1700000000,"object":"response"}}"#;
const GENERIC_FAILED: &str = r#"{"type":"response.failed","response":{"id":"resp_1","created_at":1700000000,"object":"response","error":{"code":"server_error","message":"boom"}}}"#;

const ITEM_ADDED: &str = r#"{"type":"response.output_item.added","output_index":0,"item":{"type":"message","id":"msg_1","container_id":"c"}}"#;
const ITEM_DONE: &str = r#"{"type":"response.output_item.done","output_index":0,"sequence_number":1,"item":{"type":"message","id":"msg_1","container_id":"c"}}"#;
const GENERIC_ITEM_ADDED: &str = r#"{"type":"response.output_item.added","item":{"type":"message","id":"msg_1","container_id":"c"}}"#;
const ANNOTATION: &str = r#"{"type":"response.output_text.annotation.added","item_id":"msg_1","output_index":0,"content_index":0,"annotation_index":0,"annotation":{"type":"container_file_citation","container_id":"c"}}"#;
const GENERIC_ANNOTATION: &str = r#"{"type":"response.output_text.annotation.added","item_id":"msg_1","output_index":0,"annotation":{"type":"container_file_citation","container_id":"c"}}"#;
const PART_TYPED: &str = r#"{"type":"response.content_part.done","item_id":"msg_1","output_index":0,"content_index":0,"part":{"type":"output_text","text":"hi","annotations":[{"type":"container_file_citation","container_id":"c"}],"logprobs":null}}"#;
const PART_DICT: &str = r#"{"type":"response.content_part.done","item_id":"msg_1","output_index":0,"content_index":0,"part":{"type":"output_text","text":"hi","annotations":[{"type":"container_file_citation","container_id":"c"}]}}"#;
const PART_REFUSAL: &str = r#"{"type":"response.content_part.done","item_id":"msg_1","output_index":0,"content_index":0,"part":{"type":"refusal","refusal":"no"}}"#;

const CIPHER_ADDED: &str = r#"{"type":"response.output_item.added","output_index":0,"item":{"type":"reasoning","id":"rs_1","encrypted_content":"cipher"}}"#;
const CIPHER_DONE: &str = r#"{"type":"response.output_item.done","output_index":0,"sequence_number":1,"item":{"type":"reasoning","id":"rs_1","encrypted_content":"cipher"}}"#;
const CIPHER_GENERIC_ITEM: &str = r#"{"type":"response.output_item.added","item":{"type":"reasoning","encrypted_content":"cipher"}}"#;
const CIPHER_ON_DELTA: &str = r#"{"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"content_index":0,"delta":"hi","item":{"type":"reasoning","encrypted_content":"cipher"}}"#;
const CIPHER_EMPTY: &str = r#"{"type":"response.output_item.added","output_index":0,"item":{"type":"reasoning","id":"rs_1","encrypted_content":""}}"#;
const CIPHER_NUMBER: &str = r#"{"type":"response.output_item.added","output_index":0,"item":{"type":"reasoning","id":"rs_1","encrypted_content":5}}"#;
const CIPHER_ALREADY_WRAPPED: &str = r#"{"type":"response.output_item.added","output_index":0,"item":{"type":"reasoning","id":"rs_1","encrypted_content":"litellm_enc:bW9kZWxfaWQ6ZGVwbG95bWVudF9i;cipher"}}"#;

const UNRELATED: &str =
    r#"{"type":"response.file_search_call.completed","output_index":0,"item_id":"fs_1"}"#;

fn timestamp(text: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(text).expect("the fixture time parses")
}

fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds")
}

fn decode(payload: &str) -> ResponsesAPIStreamingResponse {
    serde_json::from_str(payload).expect("the fixture event decodes")
}

fn event_json(event: &ResponsesAPIStreamingResponse) -> Json {
    serde_json::to_value(event)
        .ok()
        .and_then(|value| serde_json::from_value::<Json>(value).ok())
        .unwrap_or(Json::Null)
}

fn at<'json>(json: &'json Json, names: &[&str]) -> &'json Json {
    names.iter().fold(json, |current, name| match current {
        Json::Object(fields) => fields.get(*name).unwrap_or(&Json::Null),
        Json::List(items) => name
            .parse::<usize>()
            .ok()
            .and_then(|index| items.get(index))
            .unwrap_or(&Json::Null),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => &Json::Null,
    })
}

fn number_text(value: &Json) -> String {
    match value {
        Json::Number(number) => number.to_string(),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => {
            String::new()
        }
    }
}

fn metadata(model_id: Json, enabled: Json) -> IndexMap<String, Json> {
    let mut info = IndexMap::new();
    info.insert("id".to_owned(), model_id);
    let mut fields = IndexMap::new();
    fields.insert("model_info".to_owned(), Json::Object(info));
    fields.insert("encrypted_content_affinity_enabled".to_owned(), enabled);
    fields
}

fn stream_state() -> ResponsesStreamBaseState {
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

fn affine_state(enabled: bool, model_id: Option<&str>) -> ResponsesStreamBaseState {
    let mut state = stream_state();
    state.litellm_metadata = Some(metadata(
        Json::Text(model_id.unwrap_or_default().to_owned()),
        Json::Bool(enabled),
    ));
    state.hidden_params.model_id = model_id.map(str::to_owned);
    state
}

fn model_state(model_id: &str) -> ResponsesStreamBaseState {
    let mut state = stream_state();
    state.hidden_params.model_id = Some(model_id.to_owned());
    state
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

fn completion_start(logging: &StreamLoggingObject) -> Option<DateTime<FixedOffset>> {
    logging
        .read(&mut |state| state.completion_start_time)
        .flatten()
}

struct FixedClock(DateTime<FixedOffset>);

impl SuccessClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        self.0
    }
}

struct TickingClock {
    calls: Mutex<i64>,
    start: DateTime<FixedOffset>,
}

impl SuccessClock for TickingClock {
    fn now(&self) -> DateTime<FixedOffset> {
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        let next = calls.saturating_add(1);
        *calls = next;
        self.start + chrono::TimeDelta::seconds(next)
    }
}

fn fixed_clock() -> Arc<dyn SuccessClock> {
    Arc::new(FixedClock(timestamp(CLOCK)))
}

#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<String>>>);

impl Recorder {
    fn push(&self, label: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(label.to_owned());
    }

    fn entries(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Clone, Default)]
struct Seen(Arc<Mutex<Vec<(String, Json)>>>);

impl Seen {
    fn push(&self, model: &str, chunk: &Json) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((model.to_owned(), chunk.clone()));
    }

    fn entries(&self) -> Vec<(String, Json)> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Default)]
struct SuccessPort {
    calls: Recorder,
    dispatches: Recorder,
    payloads: Mutex<Vec<Json>>,
}

impl SuccessPort {
    fn payloads(&self) -> Vec<Json> {
        self.payloads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, request: &SuccessRequest) {
        self.payloads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.result.response().clone());
    }
}

#[async_trait]
impl ResponsesStreamSuccessTarget for SuccessPort {
    async fn dispatch_success_handlers(&self, request: SuccessRequest) {
        self.dispatches.push("dispatch");
        self.record(&request);
    }

    async fn async_success_handler(&self, request: &SuccessRequest) {
        self.calls.push("async");
        self.record(request);
    }

    fn success_handler(&self, request: &SuccessRequest) {
        self.calls.push("sync");
        self.record(request);
    }

    fn deferred_armed(&self) -> bool {
        false
    }
}

#[derive(Default)]
struct FailurePort {
    calls: Recorder,
    deliveries: Mutex<Vec<String>>,
}

impl FailurePort {
    fn deliveries(&self) -> Vec<String> {
        self.deliveries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, delivery: &FailureDelivery, label: &str) {
        self.calls.push(label);
        self.deliveries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("{:?}", delivery.exception));
    }
}

#[async_trait]
impl ResponsesStreamFailureTarget for FailurePort {
    async fn async_failure_handler(&self, delivery: &FailureDelivery) -> Result<(), PortFailure> {
        self.record(delivery, "async");
        Ok(())
    }

    fn failure_handler(&self, delivery: &FailureDelivery) -> Result<(), PortFailure> {
        self.record(delivery, "sync");
        Ok(())
    }
}

struct FailingCache;

#[async_trait]
impl ResponsesStreamCacheTarget for FailingCache {
    fn add_cache(&self, _write: CacheWrite) -> Result<(), PortFailure> {
        Err(PortFailure {
            message: "cache write failed".to_owned(),
        })
    }

    async fn async_add_cache(&self, _write: CacheWrite) -> Result<(), PortFailure> {
        Err(PortFailure {
            message: "cache write failed".to_owned(),
        })
    }
}

struct StreamCacheHandler {
    kwargs: IndexMap<String, Json>,
}

impl ResponsesStreamCacheHandler for StreamCacheHandler {
    fn request_kwargs(&self) -> Option<&IndexMap<String, Json>> {
        Some(&self.kwargs)
    }

    fn preset_cache_key(&self) -> Option<&str> {
        None
    }

    fn original_function(&self) -> Option<&str> {
        Some("aresponses")
    }

    fn dual_cache(&self) -> Option<crate::api::responses_stream_hooks::ports::DynamicCacheObject> {
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

struct Ports {
    success: Arc<SuccessPort>,
    failure: Arc<FailurePort>,
    cache: Option<Arc<dyn ResponsesStreamCacheTarget>>,
    cache_handler: Option<Arc<dyn ResponsesStreamCacheHandler>>,
}

impl Ports {
    fn recording() -> Self {
        Self {
            success: Arc::new(SuccessPort::default()),
            failure: Arc::new(FailurePort::default()),
            cache: None,
            cache_handler: None,
        }
    }

    fn failing_cache() -> Self {
        let mut kwargs = IndexMap::new();
        kwargs.insert("stream".to_owned(), Json::Bool(true));
        Self {
            success: Arc::new(SuccessPort::default()),
            failure: Arc::new(FailurePort::default()),
            cache: Some(Arc::new(FailingCache)),
            cache_handler: Some(Arc::new(StreamCacheHandler { kwargs })),
        }
    }

    fn hooks<'run>(
        &self,
        redactor: &'run Redactor,
        clock: Arc<dyn SuccessClock>,
    ) -> ResponsesStreamHooks<'run> {
        let success: Arc<dyn ResponsesStreamSuccessTarget> = Arc::clone(&self.success);
        let failure: Arc<dyn ResponsesStreamFailureTarget> = Arc::clone(&self.failure);
        ResponsesStreamHooks {
            cache: self.cache.clone(),
            cache_handler: self.cache_handler.clone(),
            success: Some(success),
            failure: Some(failure),
            metadata: None,
            success_hook: None,
            chunk_hooks: Vec::new(),
            clock,
            redactor,
        }
    }
}

enum TransformMode {
    Direct,
    Rejecting,
    Recording(Seen),
    Fixed(ResponsesAPIStreamingResponse),
}

impl TransformMode {
    fn recording() -> (Self, Seen) {
        let seen = Seen::default();
        (Self::Recording(seen.clone()), seen)
    }
}

impl ResponsesEventTransform for TransformMode {
    type Error = IoError;

    fn transform(
        &self,
        model: &str,
        chunk: &Json,
        logging: &mut IndexMap<String, Json>,
    ) -> Result<ResponsesAPIStreamingResponse, IoError> {
        match self {
            Self::Direct => {
                let encoded = serde_json::to_string(chunk).map_err(IoError::other)?;
                match serde_json::from_str::<ResponsesAPIStreamingResponse>(&encoded) {
                    Ok(event) => Ok(event),
                    Err(_error) => Ok(generic_event(chunk)),
                }
            }
            Self::Rejecting => Err(IoError::other("the provider transform failed")),
            Self::Recording(seen) => {
                seen.push(model, chunk);
                logging.insert("transform_type".to_owned(), at(chunk, &["type"]).clone());
                Ok(decode(TEXT_DELTA))
            }
            Self::Fixed(event) => Ok(event.clone()),
        }
    }
}

fn generic_event(chunk: &Json) -> ResponsesAPIStreamingResponse {
    let mut extra = Extra::new();
    let mut event_type = "None".to_owned();
    if let Json::Object(fields) = chunk {
        for (name, value) in fields {
            match (name.as_str(), value) {
                ("type", Json::Text(text)) => event_type.clone_from(text),
                ("type", _) => {}
                (_, value) => {
                    extra.insert(name.clone(), value.clone());
                }
            }
        }
    }
    ResponsesAPIStreamingResponse::Generic(GenericEvent {
        r#type: event_type,
        extra,
    })
}

#[derive(Clone)]
struct EstimateCall {
    model: String,
    request_input: Json,
    request: Json,
    generated: String,
}

struct Tokenization {
    input: i64,
    output: i64,
    calls: Mutex<Vec<EstimateCall>>,
}

impl Tokenization {
    fn new(input: i64, output: i64) -> Self {
        Self {
            input,
            output,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<EstimateCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl UsageTokenization for Tokenization {
    type Error = IoError;

    fn messages(&self, request: &EstimateRequest<'_>) -> Result<Vec<Json>, IoError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(EstimateCall {
                model: request.model.to_owned(),
                request_input: request.request_input.clone(),
                request: request.responses_api_request.clone(),
                generated: request.generated_text.to_owned(),
            });
        Ok(vec![Json::Text("messages".to_owned())])
    }

    fn count(&self, request: TokenCountRequest<'_>) -> Result<i64, IoError> {
        Ok(if request.count_response_tokens {
            self.output
        } else {
            self.input
        })
    }
}

struct Calculator {
    cost: Option<Decimal>,
    calls: Mutex<usize>,
}

impl Calculator {
    fn charging(cost: &str) -> Self {
        Self {
            cost: Some(Decimal::from_str(cost).expect("the fixture cost parses")),
            calls: Mutex::new(0),
        }
    }

    fn missing() -> Self {
        Self {
            cost: None,
            calls: Mutex::new(0),
        }
    }

    fn calls(&self) -> usize {
        *self.calls.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl ResponsesCostCalculator for Calculator {
    type Error = IoError;

    fn response_cost(&self, _target: &CostTarget<'_>) -> Result<Option<Decimal>, IoError> {
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        let next = calls.saturating_add(1);
        *calls = next;
        Ok(self.cost)
    }
}

struct QuotaCounter {
    calls: Mutex<usize>,
}

#[async_trait]
impl ProjectQuotaCallback for QuotaCounter {
    async fn enforce_frame_quota(
        &self,
        _user_api_key_dict: Option<&UserAPIKeyAuth>,
        _requested_model: Option<&str>,
        _estimated_input_tokens: i64,
        _estimated_output_tokens: i64,
    ) {
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        let next = calls.saturating_add(1);
        *calls = next;
    }
}

impl QuotaCounter {
    fn calls(&self) -> usize {
        *self.calls.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

struct Rig<T> {
    ports: Ports,
    redactor: Redactor,
    transform: Option<T>,
    tokenization: Option<Tokenization>,
    calculator: Option<Calculator>,
    limits: ApiLimits,
    details: IndexMap<String, Json>,
    state: ResponsesStreamBaseState,
    callback: ResponsesStreamHookState,
    logging: StreamLoggingObject,
    clock: Arc<dyn SuccessClock>,
}

impl<T: ResponsesEventTransform> Rig<T> {
    fn new(transform: Option<T>) -> Self {
        Self::with_ports(Ports::recording(), transform)
    }

    fn with_ports(ports: Ports, transform: Option<T>) -> Self {
        Self {
            ports,
            redactor: redactor(),
            transform,
            tokenization: None,
            calculator: None,
            limits: ApiLimits::default(),
            details: IndexMap::new(),
            state: stream_state(),
            callback: ResponsesStreamHookState::default(),
            logging: logging_object(),
            clock: fixed_clock(),
        }
    }

    async fn process(
        &mut self,
        payload: &str,
    ) -> Result<Option<ResponsesAPIStreamingResponse>, ProcessEventError<T::Error>> {
        let hooks = self.ports.hooks(&self.redactor, Arc::clone(&self.clock));
        let mut processor = ResponsesEventProcessor {
            transform: self.transform.as_ref(),
            tokenization: self.tokenization.as_ref(),
            calculator: self.calculator.as_ref(),
            hooks: &hooks,
            details: &mut self.details,
            dispatch: Dispatch::Inline,
            limits: &self.limits,
        };
        processor
            .process(payload, &mut self.state, &mut self.callback, &self.logging)
            .await
    }
}

mod framed;
mod identity;
mod terminal;
