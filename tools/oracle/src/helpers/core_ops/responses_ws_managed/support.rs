use std::collections::{BTreeMap, VecDeque};
use std::fmt::Debug;
use std::mem::take;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_core::api::response_synthetic_events::error::SyntheticEventsError;
use betterlite_core::api::response_synthetic_events::model_dump::HasModelDumpJson;
use betterlite_core::api::responses_stream_events::ProjectQuotaCallback;
use betterlite_core::api::responses_ws_history::error::FrameSinkError;
use betterlite_core::api::responses_ws_history::provider::{
    ProviderResolution, ProviderResolutionFailure,
};
use betterlite_core::api::responses_ws_history::serialization::serialize_chunk;
use betterlite_core::api::responses_ws_history::{
    Connection, FrameSink, ManagedResponsesWebSocket, WsDeps,
};
use betterlite_core::api::responses_ws_managed::provider::{
    ProviderCallError, ProviderChunk, ResponsesStream, ResponsesStreaming,
};
use betterlite_core::api::responses_ws_managed::run::{FrameReceiveError, FrameSource};
use betterlite_error::error::details::RateLimitDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_error::rate_limit::RateLimitCategory;
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamSuccessCall, StreamSuccessDispatch,
};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Map, Value, from_str, json};
use tracing::field::Visit;
use tracing::{Event, Field, Subscriber};
use tracing_subscriber::layer::{Context as LayerContext, Layer};

use super::Setup;

const CLOCK_SECONDS: i64 = 1_765_000_000;
const WARMUP_ID_HEX: &str = "0123456789abcdef0123456789abcdef";
const DIAGNOSTIC_TARGET: &str = "betterlite_core::api::responses_ws_";
const DIAGNOSTIC_PREFIX: &str = "ManagedResponsesWS: ";
const SCRIPT_EXHAUSTED: &str = "the case scripts no further provider call";
const BROKEN_DUMP: &str = "chunk must be a JSON object";

#[derive(Clone, Deserialize)]
pub(super) struct ChunkRow {
    pub(super) kind: String,
    pub(super) value: Option<Json>,
    pub(super) event_type: Option<String>,
    pub(super) dump: Option<String>,
}

#[derive(Clone, Deserialize)]
pub(super) struct ScriptRow {
    pub(super) fail: Option<String>,
    pub(super) chunks: Vec<ChunkRow>,
    pub(super) raise_after: Option<usize>,
    pub(super) raise_message: String,
}

#[derive(Clone, Deserialize)]
pub(super) struct QuotaRow {
    pub(super) kind: String,
    pub(super) message: String,
}

#[derive(Deserialize)]
pub(super) struct SinkRow {
    pub(super) fails_at: Option<usize>,
    pub(super) message: String,
}

#[derive(Default)]
pub(super) struct Recorder {
    pub(super) frames: Vec<String>,
    pub(super) sends: usize,
    pub(super) calls: Vec<RecordedCall>,
    pub(super) quotas: Vec<RecordedQuota>,
    pub(super) trace: Vec<String>,
}

pub(super) struct RecordedCall {
    pub(super) model: String,
    pub(super) kwargs: IndexMap<String, Json>,
}

pub(super) struct RecordedQuota {
    pub(super) model: Option<String>,
    pub(super) input_tokens: i64,
    pub(super) output_tokens: i64,
}

pub(super) type Shared = Arc<Mutex<Recorder>>;

pub(super) fn recorder() -> Shared {
    Arc::new(Mutex::new(Recorder::default()))
}

pub(super) fn lock<Value>(shared: &Mutex<Value>) -> MutexGuard<'_, Value> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(super) struct Socket {
    shared: Shared,
    fails_at: Option<usize>,
    message: String,
}

#[async_trait]
impl FrameSink for Socket {
    async fn send_text(&self, frame: &str) -> Result<(), FrameSinkError> {
        let mut recorder = lock(&self.shared);
        recorder.sends = recorder.sends.saturating_add(1);
        let attempt = recorder.sends;
        if Some(attempt) == self.fails_at {
            return Err(FrameSinkError {
                message: self.message.clone(),
            });
        }
        recorder.frames.push(frame.to_owned());
        Ok(())
    }
}

pub(super) fn socket(shared: &Shared, row: &SinkRow) -> Socket {
    Socket {
        shared: Arc::clone(shared),
        fails_at: row.fails_at,
        message: row.message.clone(),
    }
}

pub(super) struct ScriptedSource {
    shared: Shared,
    frames: Vec<String>,
    disconnect: String,
    next: usize,
}

impl ScriptedSource {
    pub(super) fn new(shared: &Shared, frames: &[String], disconnect: &str) -> Self {
        Self {
            shared: Arc::clone(shared),
            frames: frames.to_vec(),
            disconnect: disconnect.to_owned(),
            next: 0,
        }
    }
}

#[async_trait]
impl FrameSource for ScriptedSource {
    async fn receive_text(&mut self) -> Result<String, FrameReceiveError> {
        lock(&self.shared).trace.push("receive".to_owned());
        match self.frames.get(self.next) {
            Some(frame) => {
                let received = frame.clone();
                self.next = self.next.saturating_add(1);
                Ok(received)
            }
            None => Err(FrameReceiveError {
                message: self.disconnect.clone(),
            }),
        }
    }
}

struct TextDump(String);

impl HasModelDumpJson for TextDump {
    fn model_dump_json(&self) -> Result<String, SyntheticEventsError> {
        Ok(self.0.clone())
    }
}

struct BrokenDump;

impl HasModelDumpJson for BrokenDump {
    fn model_dump_json(&self) -> Result<String, SyntheticEventsError> {
        Err(SyntheticEventsError::NotAnObject { field: BROKEN_DUMP })
    }
}

fn chunk_of(row: ChunkRow) -> ProviderChunk {
    match row.kind.as_str() {
        "none" => ProviderChunk::Empty,
        "json" => ProviderChunk::Json(row.value.unwrap_or(Json::Null)),
        "object" => ProviderChunk::Object {
            kind: row.event_type,
            dump: Box::new(TextDump(row.dump.unwrap_or_default())),
        },
        _broken => ProviderChunk::Object {
            kind: row.event_type,
            dump: Box::new(BrokenDump),
        },
    }
}

pub(super) struct ScriptedStream {
    shared: Shared,
    chunks: VecDeque<ProviderChunk>,
    raise_after: Option<usize>,
    raise_message: String,
    consumed: usize,
}

#[async_trait]
impl ResponsesStream for ScriptedStream {
    async fn next_chunk(&mut self) -> Result<Option<ProviderChunk>, ProviderCallError> {
        if Some(self.consumed) == self.raise_after {
            return Err(ProviderCallError {
                message: self.raise_message.clone(),
            });
        }
        let Some(chunk) = self.chunks.pop_front() else {
            lock(&self.shared).trace.push("provider-end".to_owned());
            return Ok(None);
        };
        self.consumed = self.consumed.saturating_add(1);
        Ok(Some(chunk))
    }
}

pub(super) struct ScriptedProvider {
    shared: Shared,
    scripts: Mutex<VecDeque<ScriptRow>>,
}

impl ScriptedProvider {
    pub(super) fn new(shared: &Shared, scripts: &[ScriptRow]) -> Self {
        Self {
            shared: Arc::clone(shared),
            scripts: Mutex::new(VecDeque::from(scripts.to_vec())),
        }
    }
}

#[async_trait]
impl ResponsesStreaming for ScriptedProvider {
    type Stream = ScriptedStream;

    async fn stream(
        &self,
        model: &str,
        call_kwargs: &IndexMap<String, Json>,
    ) -> Result<ScriptedStream, ProviderCallError> {
        {
            let mut recorder = lock(&self.shared);
            recorder.trace.push("provider".to_owned());
            recorder.calls.push(RecordedCall {
                model: model.to_owned(),
                kwargs: call_kwargs.clone(),
            });
        }
        let Some(row) = lock(&self.scripts).pop_front() else {
            return Err(ProviderCallError {
                message: SCRIPT_EXHAUSTED.to_owned(),
            });
        };
        if let Some(message) = row.fail {
            return Err(ProviderCallError { message });
        }
        let chunks: VecDeque<ProviderChunk> = row.chunks.into_iter().map(chunk_of).collect();
        Ok(ScriptedStream {
            shared: Arc::clone(&self.shared),
            chunks,
            raise_after: row.raise_after,
            raise_message: row.raise_message,
            consumed: 0,
        })
    }
}

pub(super) struct ScriptedQuota {
    shared: Shared,
    row: QuotaRow,
}

impl ScriptedQuota {
    pub(super) fn new(shared: &Shared, row: QuotaRow) -> Self {
        Self {
            shared: Arc::clone(shared),
            row,
        }
    }
}

#[async_trait]
impl ProjectQuotaCallback for ScriptedQuota {
    async fn enforce_frame_quota(
        &self,
        _user_api_key_dict: Option<&UserAPIKeyAuth>,
        requested_model: Option<&str>,
        estimated_input_tokens: i64,
        estimated_output_tokens: i64,
    ) -> Result<(), ErrorClass> {
        {
            let mut recorder = lock(&self.shared);
            recorder.trace.push("quota".to_owned());
            recorder.quotas.push(RecordedQuota {
                model: requested_model.map(str::to_owned),
                input_tokens: estimated_input_tokens,
                output_tokens: estimated_output_tokens,
            });
        }
        match self.row.kind.as_str() {
            "rate_limit" => Err(rate_limited(&self.row.message)),
            "internal" => Err(internal_server(&self.row.message)),
            _accept => Ok(()),
        }
    }
}

fn context(message: &str) -> ErrorContext {
    ErrorContext {
        message: message.to_owned(),
        model: None,
        provider: None,
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }
}

fn rate_limited(message: &str) -> ErrorClass {
    ErrorClass::RateLimit(Box::new(RateLimitDetails {
        context: context(message),
        category: RateLimitCategory::LitellmRateLimit,
        dimension: None,
        headers: BTreeMap::new(),
    }))
}

fn internal_server(message: &str) -> ErrorClass {
    ErrorClass::InternalServer(Box::new(context(message)))
}

struct ScriptedResolver;

impl ProviderResolution for ScriptedResolver {
    fn resolve(&self, model: &str) -> Result<String, ProviderResolutionFailure> {
        match model.split_once('/') {
            Some((provider @ ("openai" | "anthropic" | "vertex_ai"), _rest)) => {
                Ok(provider.to_owned())
            }
            _ => Err(ProviderResolutionFailure {
                message: format!("no provider answers {model}"),
            }),
        }
    }
}

struct NoDispatch;

#[async_trait]
impl StreamSuccessDispatch for NoDispatch {
    async fn dispatch_success(&self, _call: StreamSuccessCall<'_>) {}
}

pub(super) fn build_handler(
    setup: &Setup,
    socket: Socket,
    shared: &Shared,
    quota: Option<&QuotaRow>,
) -> ManagedResponsesWebSocket<Socket> {
    let callbacks: Option<Vec<Arc<dyn ProjectQuotaCallback>>> =
        quota.map(|row| vec![Arc::new(ScriptedQuota::new(shared, row.clone()))]);
    let connection = Connection {
        websocket: socket,
        model: setup.model.clone(),
        logging_obj: Arc::new(StreamLoggingObject {
            slot: Mutex::new(None),
            dispatch: Arc::new(NoDispatch),
        }),
        user_api_key_dict: None,
        litellm_metadata: setup.litellm_metadata.clone(),
        api_key: setup.api_key.clone().map(SecretString::from),
        api_base: setup.api_base.clone(),
        timeout: setup.timeout,
        custom_llm_provider: setup.custom_llm_provider.clone(),
        first_message: setup.first_message.clone(),
        quota_callbacks: callbacks,
        kwargs: setup.kwargs.clone(),
    };
    ManagedResponsesWebSocket::new(
        connection,
        WsDeps {
            resolver: Arc::new(ScriptedResolver),
            clock: Arc::new(|| CLOCK_SECONDS),
            ids: Arc::new(|| WARMUP_ID_HEX.to_owned()),
            serializer: Arc::new(serialize_chunk),
        },
    )
}

pub(super) fn frames(shared: &Shared) -> Result<Value, Box<HelperError>> {
    let recorder = lock(shared);
    let mut parsed = Vec::with_capacity(recorder.frames.len());
    for frame in &recorder.frames {
        let value = from_str::<Value>(frame)
            .map_err(|error| helper_error("ValueError", error.to_string()))?;
        parsed.push(value);
    }
    Ok(Value::Array(parsed))
}

pub(super) fn calls(shared: &Shared) -> Value {
    let recorder = lock(shared);
    let rows: Vec<Value> = recorder
        .calls
        .iter()
        .map(|call| json!({"model": call.model, "kwargs": call.kwargs}))
        .collect();
    Value::Array(rows)
}

pub(super) fn quotas(shared: &Shared) -> Value {
    let recorder = lock(shared);
    let rows: Vec<Value> = recorder
        .quotas
        .iter()
        .map(|quota| {
            json!({
                "model": quota.model,
                "input_tokens": quota.input_tokens,
                "output_tokens": quota.output_tokens,
            })
        })
        .collect();
    Value::Array(rows)
}

pub(super) fn trace(shared: &Shared) -> Value {
    json!(lock(shared).trace)
}

pub(super) fn history<Sink: FrameSink>(handler: &ManagedResponsesWebSocket<Sink>) -> Value {
    let mut stored = Map::new();
    for (response_id, messages) in &handler.session_history {
        stored.insert(response_id.clone(), json!(messages));
    }
    Value::Object(stored)
}

pub(super) fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message,
    })
}

#[derive(Clone, Default)]
pub(super) struct Captured(Arc<Mutex<Vec<String>>>);

impl Captured {
    pub(super) fn take(&self) -> Value {
        let mut messages = lock(&self.0);
        json!(take(&mut *messages))
    }
}

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0 = value.to_owned();
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Captured {
    fn on_event(&self, event: &Event<'_>, _context: LayerContext<'_, Inner>) {
        if !event.metadata().target().starts_with(DIAGNOSTIC_TARGET) {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        if !message.0.starts_with(DIAGNOSTIC_PREFIX) {
            return;
        }
        lock(&self.0).push(message.0);
    }
}
