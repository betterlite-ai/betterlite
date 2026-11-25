use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_core::cost::recording::PortFailure;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_logging_state::logger::PassThroughEndpointLogging;
use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamLoggingState, StreamSuccessCall, StreamSuccessDispatch,
};
use betterlite_logging::passthrough_stream_logging::types::{
    CollectedChunksCall, EndpointType, HandlerOutcome,
};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, TimeZone as _, Utc};
use futures_util::future::BoxFuture;
use http::HeaderMap;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use super::super::{
    AnthropicPassthroughError, AnthropicPassthroughInput, AnthropicPassthroughLogger,
};
use crate::pass_through_logging::anthropic_batch::{
    AnthropicBatchLogging, BatchRegistration, ManagedBatchRegistry, RegistryError, ResponseIds,
};
use crate::pass_through_logging::response_costing::{ResponseCostRequest, ResponseCostService};

pub(super) const MESSAGES_ROUTE: &str = "/anthropic/v1/messages";
pub(super) const CALL_ID: &str = "call-1";
pub(super) const BATCH_RESPONSE_ID: &str = "batch-response-1";

pub(super) const MESSAGE: &str = r#"{"id":"msg_up","type":"message","role":"assistant","model":"claude-sonnet-4-5","content":[{"type":"text","text":"hello"}],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":10,"output_tokens":5}}"#;
pub(super) const BATCH: &str = r#"{"id":"msgbatch_1","type":"message_batch","processing_status":"in_progress","request_counts":{"processing":1,"succeeded":0,"errored":0,"canceled":0,"expired":0},"created_at":"2026-01-01T00:00:00Z","expires_at":"2026-01-02T00:00:00Z","ended_at":null}"#;
pub(super) const RESULTS: &str = concat!(
    r#"{"custom_id":"a","result":{"type":"succeeded","message":{"id":"msg_a","type":"message","role":"assistant","model":"claude-sonnet-4-5","content":[{"type":"text","text":"one"}],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":10,"output_tokens":5}}}}"#,
    "\n",
    r#"{"custom_id":"b","result":{"type":"succeeded","message":{"id":"msg_b","type":"message","role":"assistant","model":"claude-sonnet-4-5","content":[{"type":"text","text":"two"}],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":3,"output_tokens":2}}}}"#,
    "\n",
);

pub(super) const STREAM: [&str; 18] = [
    "event: message_start",
    r#"data: {"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-test","content":[],"stop_reason":null,"usage":{"input_tokens":10,"output_tokens":1}}}"#,
    "",
    "event: content_block_start",
    r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
    "",
    "event: content_block_delta",
    r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello world"}}"#,
    "",
    "event: content_block_stop",
    r#"data: {"type":"content_block_stop","index":0}"#,
    "",
    "event: message_delta",
    r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":5}}"#,
    "",
    "event: message_stop",
    r#"data: {"type":"message_stop"}"#,
    "",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PricedCall {
    pub(super) model: String,
    pub(super) provider: Option<String>,
    pub(super) speed: Option<String>,
}

pub(super) struct RecordingCost {
    price: Option<Decimal>,
    calls: Mutex<Vec<PricedCall>>,
}

impl RecordingCost {
    pub(super) fn priced() -> Arc<Self> {
        Arc::new(Self {
            price: Some(Decimal::new(125, 2)),
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn failing() -> Arc<Self> {
        Arc::new(Self {
            price: None,
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn calls(&self) -> Vec<PricedCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ResponseCostService for RecordingCost {
    fn completion_cost(&self, request: &ResponseCostRequest<'_>) -> Result<Decimal, PortFailure> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(PricedCall {
                model: request.model.to_owned(),
                provider: request.custom_llm_provider.map(str::to_owned),
                speed: text_at(request.response, &["usage", "speed"]),
            });
        self.price
            .ok_or_else(|| PortFailure::from("no price for this model"))
    }
}

#[derive(Default)]
pub(super) struct RecordingRegistry {
    registrations: Mutex<Vec<BatchRegistration>>,
}

impl RecordingRegistry {
    pub(super) fn registrations(&self) -> Vec<BatchRegistration> {
        self.registrations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ManagedBatchRegistry for RecordingRegistry {
    fn store_unified_object_id(
        &self,
        request: BatchRegistration,
    ) -> BoxFuture<'static, Result<(), RegistryError>> {
        self.registrations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        Box::pin(async { Ok(()) })
    }
}

struct FixedIds;

impl ResponseIds for FixedIds {
    fn new_response_id(&self) -> String {
        BATCH_RESPONSE_ID.to_owned()
    }
}

pub(super) fn logger(
    cost: &Arc<RecordingCost>,
    registry: &Arc<RecordingRegistry>,
) -> AnthropicPassthroughLogger {
    let shared_registry: Arc<dyn ManagedBatchRegistry> = Arc::<RecordingRegistry>::clone(registry);
    let batch_logging = AnthropicBatchLogging::new(None, Some(shared_registry), Arc::new(FixedIds));
    let shared_cost: Arc<dyn ResponseCostService> = Arc::<RecordingCost>::clone(cost);
    AnthropicPassthroughLogger::new(batch_logging, shared_cost)
}

pub(super) struct Exchange<'exchange> {
    pub(super) status_code: u16,
    pub(super) url_route: &'exchange str,
    pub(super) response_text: &'exchange str,
    pub(super) request_body: JsonObject,
    pub(super) kwargs: PassThroughLoggingKwargs,
}

impl<'exchange> Exchange<'exchange> {
    pub(super) fn message(url_route: &'exchange str) -> Self {
        Self {
            status_code: 200,
            url_route,
            response_text: MESSAGE,
            request_body: JsonObject::new(),
            kwargs: PassThroughLoggingKwargs::default(),
        }
    }

    pub(super) fn handle(
        self,
        logger: &AnthropicPassthroughLogger,
        call_log: &mut LoggingState,
    ) -> Result<HandlerOutcome, AnthropicPassthroughError> {
        let headers = HeaderMap::new();
        // B3606 hands over an empty object when the body is a list or no JSON at all
        let response_body: JsonObject =
            serde_json::from_str(self.response_text).unwrap_or_default();
        let input = AnthropicPassthroughInput {
            status_code: self.status_code,
            response_headers: &headers,
            response_text: self.response_text,
            response_body: &response_body,
            url_route: self.url_route,
            start_time: start_time(),
            end_time: start_time(),
            request_body: &self.request_body,
            kwargs: self.kwargs,
        };
        logger.anthropic_passthrough_handler(input, call_log)
    }
}

// The batch branch's inputs: a batch create route, the batch body and the metadata kwargs the registry reads.
pub(super) fn batch_create_exchange() -> Exchange<'static> {
    let mut exchange = Exchange::message("/anthropic/v1/messages/batches");
    exchange.response_text = BATCH;
    exchange.request_body = object(&[("model", text("claude-sonnet-4-5"))]);
    let metadata = object(&[("user_api_key_user_id", text("user-7"))]);
    exchange.kwargs = kwargs(&[
        ("call_type", text("pass_through_endpoint")),
        (
            "litellm_params",
            Json::Object(object(&[("metadata", Json::Object(metadata))])),
        ),
    ]);
    exchange
}

// The stream seam's call over the case's lines, with the case's body as the request.
pub(super) fn collected_call<'call>(
    logging_obj: &'call StreamLoggingObject,
    passthrough: &'call PassThroughEndpointLogging,
    lines: &'call [String],
    request_body: Option<&'call Json>,
    model: Option<&'call str>,
) -> CollectedChunksCall<'call> {
    CollectedChunksCall {
        logging_obj,
        passthrough,
        url_route: MESSAGES_ROUTE,
        request_body,
        endpoint_type: EndpointType::Anthropic,
        start_time: start_time().fixed_offset(),
        end_time: start_time().fixed_offset(),
        lines,
        model,
    }
}

pub(super) fn start_time() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
        .single()
        .expect("a valid start time")
}

pub(super) fn call_log(details: &[(&str, Json)]) -> LoggingState {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("a valid redactor");
    let mut state = LoggingState::new(
        LoggingInit {
            model: String::new(),
            call_type: "pass_through_endpoint".to_owned(),
            start_time: start_time().fixed_offset(),
            litellm_call_id: CALL_ID.to_owned(),
            function_id: "function-1".to_owned(),
            messages: None,
            stream: Some(false),
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
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || "generated-trace".to_owned(),
        || Duration::ZERO,
    )
    .expect("a valid logging state");
    for (name, value) in details {
        state
            .model_call_details
            .insert((*name).to_owned(), value.clone());
    }
    state
}

struct NoDispatch;

#[async_trait]
impl StreamSuccessDispatch for NoDispatch {
    async fn dispatch_success(&self, _call: StreamSuccessCall<'_>) {}
}

pub(super) fn logging_object(state: Option<LoggingState>) -> StreamLoggingObject {
    StreamLoggingObject {
        slot: Mutex::new(state.map(|logging| StreamLoggingState {
            logging,
            correlation: CorrelationContext::default(),
        })),
        dispatch: Arc::new(NoDispatch),
    }
}

pub(super) fn object(entries: &[(&str, Json)]) -> JsonObject {
    let mut fields = JsonObject::new();
    for (name, value) in entries {
        fields.insert((*name).to_owned(), value.clone());
    }
    fields
}

pub(super) fn kwargs(entries: &[(&str, Json)]) -> PassThroughLoggingKwargs {
    PassThroughLoggingKwargs {
        entries: object(entries),
    }
}

pub(super) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(super) fn at<'json>(value: &'json Json, path: &[&str]) -> Option<&'json Json> {
    path.iter().try_fold(value, |current, key| {
        if let Json::Object(fields) = current {
            fields.get(*key)
        } else {
            None
        }
    })
}

pub(super) fn text_at(value: &Json, path: &[&str]) -> Option<String> {
    if let Some(Json::Text(found)) = at(value, path) {
        Some(found.clone())
    } else {
        None
    }
}

pub(super) fn stored_complete_response(logging_obj: &StreamLoggingObject) -> Option<Json> {
    logging_obj
        .read(&mut |state| {
            state
                .model_call_details
                .get("complete_streaming_response")
                .cloned()
        })
        .flatten()
}

pub(super) fn model_response(outcome: &HandlerOutcome) -> &Json {
    let Some(PassThroughResponse::ModelResponse(body)) = &outcome.result else {
        panic!("expected a model response, got {:?}", outcome.result);
    };
    body
}

#[derive(Clone, Default)]
pub(super) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub(super) fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8(bytes.clone()).expect("the captured log lines are UTF-8")
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

pub(super) fn capture_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_target(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}
