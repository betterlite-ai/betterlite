use std::collections::{BTreeMap, VecDeque};
use std::fmt::Debug;
use std::future::Future;
use std::mem::take;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_error::error::details::RateLimitDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_error::rate_limit::RateLimitCategory;
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamSuccessCall, StreamSuccessDispatch,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use thiserror::Error;
use tracing::field::Field;
use tracing::field::Visit;
use tracing::instrument::WithSubscriber as _;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context as LayerContext, Layer, SubscriberExt as _};

use crate::api::response_synthetic_events::error::SyntheticEventsError;
use crate::api::response_synthetic_events::model_dump::HasModelDumpJson;
use crate::api::responses_stream_events::ProjectQuotaCallback;
use crate::api::responses_ws_history::error::FrameSinkError;
use crate::api::responses_ws_history::provider::{ProviderResolution, ProviderResolutionFailure};
use crate::api::responses_ws_history::serialization::serialize_chunk;
use crate::api::responses_ws_history::{Connection, FrameSink, ManagedResponsesWebSocket, WsDeps};
use crate::api::responses_ws_managed::provider::{
    ProviderCallError, ProviderChunk, ResponsesStream, ResponsesStreaming,
};
use crate::api::responses_ws_managed::run::{FrameReceiveError, FrameSource};

pub(super) const MODEL: &str = "openai/gpt-4o-mini";
pub(super) const GROUP: &str = "router-model";
pub(super) const NOW: i64 = 1_765_000_000;
pub(super) const WARMUP_ID: &str = "0123456789abcdef0123456789abcdef";
pub(super) const SEND_FAILED: &str = "client send failed";

const DIAGNOSTIC_TARGET: &str = "betterlite_core::api::responses_ws_";
const DIAGNOSTIC_PREFIX: &str = "ManagedResponsesWS: ";

pub(super) type Shared = Arc<Mutex<Recorder>>;

#[derive(Default)]
pub(super) struct Recorder {
    pub(super) frames: Vec<String>,
    pub(super) sends: usize,
    pub(super) calls: Vec<RecordedCall>,
    pub(super) quotas: Vec<RecordedQuota>,
    pub(super) trace: Vec<String>,
}

#[derive(Clone)]
pub(super) struct RecordedCall {
    pub(super) model: String,
    pub(super) kwargs: IndexMap<String, Json>,
}

#[derive(Clone)]
pub(super) struct RecordedQuota {
    pub(super) model: Option<String>,
    pub(super) input_tokens: i64,
    pub(super) output_tokens: i64,
}

pub(super) fn shared() -> Shared {
    Arc::new(Mutex::new(Recorder::default()))
}

// A scripted provider or socket failure carries only the text the test scripts for it.
#[derive(Debug, Error)]
#[error("{message}")]
pub(super) struct ScriptedFailure {
    pub(super) message: String,
}

pub(super) fn lock<Value>(shared: &Mutex<Value>) -> MutexGuard<'_, Value> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Clone)]
pub(super) struct ScriptedSink {
    shared: Shared,
    fails_at: Option<usize>,
}

#[async_trait]
impl FrameSink for ScriptedSink {
    async fn send_text(&self, frame: &str) -> Result<(), FrameSinkError> {
        let attempt = self.take_attempt();
        if Some(attempt) == self.fails_at {
            return Err(FrameSinkError {
                message: "client send failed".to_owned(),
            });
        }
        lock(&self.shared).frames.push(frame.to_owned());
        Ok(())
    }
}

impl ScriptedSink {
    fn take_attempt(&self) -> usize {
        let mut recorder = lock(&self.shared);
        recorder.sends = recorder.sends.saturating_add(1);
        recorder.sends
    }
}

pub(super) fn sink(shared: &Shared) -> ScriptedSink {
    ScriptedSink {
        shared: Arc::clone(shared),
        fails_at: None,
    }
}

pub(super) fn failing_sink(shared: &Shared, fails_at: usize) -> ScriptedSink {
    ScriptedSink {
        shared: Arc::clone(shared),
        fails_at: Some(fails_at),
    }
}

pub(super) struct ScriptedSource {
    shared: Shared,
    frames: Vec<String>,
    error: String,
    next: usize,
}

impl ScriptedSource {
    pub(super) fn new(shared: &Shared, frames: &[&str], error: &str) -> Self {
        Self {
            shared: Arc::clone(shared),
            frames: frames.iter().map(|frame| (*frame).to_owned()).collect(),
            error: error.to_owned(),
            next: 0,
        }
    }
}

#[async_trait]
impl FrameSource for ScriptedSource {
    type Error = ScriptedFailure;

    async fn receive_text(&mut self) -> Result<String, FrameReceiveError<ScriptedFailure>> {
        lock(&self.shared).trace.push("receive".to_owned());
        match self.frames.get(self.next) {
            Some(frame) => {
                let received = frame.clone();
                self.next = self.next.saturating_add(1);
                Ok(received)
            }
            None => Err(FrameReceiveError::Disconnected(ScriptedFailure {
                message: self.error.clone(),
            })),
        }
    }
}

pub(super) fn source(shared: &Shared, frames: &[&str], error: &str) -> ScriptedSource {
    ScriptedSource::new(shared, frames, error)
}

pub(super) enum ChunkScript {
    Json(Json),
    Object { text: String, kind: Option<String> },
    Broken,
    Empty,
}

pub(super) fn event(text: &str) -> ChunkScript {
    ChunkScript::Json(json(text))
}

pub(super) fn typed_object(text: &str, kind: &str) -> ChunkScript {
    ChunkScript::Object {
        text: text.to_owned(),
        kind: Some(kind.to_owned()),
    }
}

pub(super) fn object_chunk(text: &str) -> ChunkScript {
    ChunkScript::Object {
        text: text.to_owned(),
        kind: None,
    }
}

pub(super) fn broken_chunk() -> ChunkScript {
    ChunkScript::Broken
}

pub(super) fn empty_chunk() -> ChunkScript {
    ChunkScript::Empty
}

pub(super) enum Script {
    Fail(&'static str),
    Stream {
        chunks: Vec<ChunkScript>,
        raise_after: Option<usize>,
        raise_message: &'static str,
    },
}

pub(super) fn script(chunks: Vec<ChunkScript>) -> Script {
    Script::Stream {
        chunks,
        raise_after: None,
        raise_message: "",
    }
}

pub(super) fn failing_script(message: &'static str) -> Script {
    Script::Fail(message)
}

pub(super) fn raising_script(
    chunks: Vec<ChunkScript>,
    after: usize,
    message: &'static str,
) -> Script {
    Script::Stream {
        chunks,
        raise_after: Some(after),
        raise_message: message,
    }
}

pub(super) struct ScriptedProvider {
    shared: Shared,
    scripts: Mutex<VecDeque<Script>>,
}

impl ScriptedProvider {
    pub(super) fn new(shared: &Shared, scripts: Vec<Script>) -> Self {
        Self {
            shared: Arc::clone(shared),
            scripts: Mutex::new(VecDeque::from(scripts)),
        }
    }

    pub(super) fn idle(shared: &Shared) -> Self {
        Self::new(shared, Vec::new())
    }

    fn record_call(&self, model: &str, call_kwargs: &IndexMap<String, Json>) {
        let mut recorder = lock(&self.shared);
        recorder.trace.push("provider".to_owned());
        recorder.calls.push(RecordedCall {
            model: model.to_owned(),
            kwargs: call_kwargs.clone(),
        });
    }
}

#[async_trait]
impl ResponsesStreaming for ScriptedProvider {
    type Error = ScriptedFailure;
    type Stream = ScriptedStream;

    async fn stream(
        &self,
        model: &str,
        call_kwargs: &IndexMap<String, Json>,
    ) -> Result<ScriptedStream, ProviderCallError<ScriptedFailure>> {
        self.record_call(model, call_kwargs);
        let Some(script) = lock(&self.scripts).pop_front() else {
            return Err(ProviderCallError::Call(ScriptedFailure {
                message: "the test scripts no further provider call".to_owned(),
            }));
        };
        match script {
            Script::Fail(message) => Err(ProviderCallError::Call(ScriptedFailure {
                message: message.to_owned(),
            })),
            Script::Stream {
                chunks,
                raise_after,
                raise_message,
            } => Ok(ScriptedStream {
                shared: Arc::clone(&self.shared),
                chunks,
                raise_after,
                raise_message: raise_message.to_owned(),
                consumed: 0,
            }),
        }
    }
}

pub(super) struct ScriptedStream {
    shared: Shared,
    chunks: Vec<ChunkScript>,
    raise_after: Option<usize>,
    raise_message: String,
    consumed: usize,
}

// the chunk a scripted object yields: its declared kind, and the text its dump returns
fn provider_chunk(script: &ChunkScript) -> ProviderChunk {
    match script {
        ChunkScript::Json(value) => ProviderChunk::Json(value.clone()),
        ChunkScript::Object { text, kind } => ProviderChunk::Object {
            kind: kind.clone(),
            dump: Box::new(TextDump(text.clone())),
        },
        ChunkScript::Broken => ProviderChunk::Object {
            kind: None,
            dump: Box::new(FailingDump),
        },
        ChunkScript::Empty => ProviderChunk::Empty,
    }
}

#[async_trait]
impl ResponsesStream for ScriptedStream {
    type Error = ScriptedFailure;

    async fn next_chunk(
        &mut self,
    ) -> Result<Option<ProviderChunk>, ProviderCallError<ScriptedFailure>> {
        if Some(self.consumed) == self.raise_after {
            return Err(ProviderCallError::Stream(ScriptedFailure {
                message: self.raise_message.clone(),
            }));
        }
        let Some(script) = self.chunks.get(self.consumed) else {
            lock(&self.shared).trace.push("provider-end".to_owned());
            return Ok(None);
        };
        let chunk = provider_chunk(script);
        self.consumed = self.consumed.saturating_add(1);
        Ok(Some(chunk))
    }
}

pub(super) struct TextDump(pub(super) String);

impl HasModelDumpJson for TextDump {
    fn model_dump_json(&self) -> Result<String, SyntheticEventsError> {
        Ok(self.0.clone())
    }
}

pub(super) struct FailingDump;

impl HasModelDumpJson for FailingDump {
    fn model_dump_json(&self) -> Result<String, SyntheticEventsError> {
        Err(SyntheticEventsError::NotAnObject { field: "chunk" })
    }
}

pub(super) enum QuotaFailure {
    RateLimit(&'static str),
    InternalServer(&'static str),
}

pub(super) struct ScriptedQuota {
    shared: Shared,
    failure: Option<QuotaFailure>,
}

impl ScriptedQuota {
    pub(super) fn new(shared: &Shared, failure: Option<QuotaFailure>) -> Self {
        Self {
            shared: Arc::clone(shared),
            failure,
        }
    }

    fn record_quota(
        &self,
        requested_model: Option<&str>,
        estimated_input_tokens: i64,
        estimated_output_tokens: i64,
    ) {
        let mut recorder = lock(&self.shared);
        recorder.trace.push("quota".to_owned());
        recorder.quotas.push(RecordedQuota {
            model: requested_model.map(str::to_owned),
            input_tokens: estimated_input_tokens,
            output_tokens: estimated_output_tokens,
        });
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
        self.record_quota(
            requested_model,
            estimated_input_tokens,
            estimated_output_tokens,
        );
        let Some(failure) = &self.failure else {
            return Ok(());
        };
        match failure {
            QuotaFailure::RateLimit(message) => Err(rate_limited(message)),
            QuotaFailure::InternalServer(message) => Err(internal_server(message)),
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

pub(super) fn connection(model: &str, sink: ScriptedSink) -> Connection<ScriptedSink> {
    Connection {
        websocket: sink,
        model: model.to_owned(),
        logging_obj: Arc::new(StreamLoggingObject {
            slot: Mutex::new(None),
            dispatch: Arc::new(NoDispatch),
        }),
        user_api_key_dict: None,
        litellm_metadata: None,
        api_key: None,
        api_base: None,
        timeout: None,
        custom_llm_provider: None,
        first_message: None,
        quota_callbacks: None,
        kwargs: IndexMap::new(),
    }
}

fn deps() -> WsDeps {
    WsDeps {
        resolver: Arc::new(ScriptedResolver),
        clock: Arc::new(|| NOW),
        ids: Arc::new(|| WARMUP_ID.to_owned()),
        serializer: Arc::new(serialize_chunk),
    }
}

pub(super) fn handler(
    connection: Connection<ScriptedSink>,
) -> ManagedResponsesWebSocket<ScriptedSink> {
    ManagedResponsesWebSocket::new(connection, deps())
}

pub(super) fn json(text: &str) -> Json {
    let parsed = serde_json::from_str::<Json>(text);
    let Ok(value) = parsed else {
        panic!("the fixture text is not JSON: {text}");
    };
    value
}

pub(super) fn object(text: &str) -> IndexMap<String, Json> {
    match json(text) {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("the fixture text is not an object: {text}")
        }
    }
}

pub(super) fn keys(kwargs: &IndexMap<String, Json>) -> Vec<String> {
    kwargs.keys().cloned().collect()
}

pub(super) fn proxy_body(kwargs: &IndexMap<String, Json>) -> IndexMap<String, Json> {
    let Some(Json::Object(request)) = kwargs.get("proxy_server_request") else {
        panic!("the call kwargs carry no proxy_server_request object");
    };
    let Some(Json::Object(body)) = request.get("body") else {
        panic!("the proxy_server_request carries no body object");
    };
    body.clone()
}

pub(super) fn warmup_ack(event_type: &str, status: &str, model: &str) -> Json {
    json(&format!(
        r#"{{"type": "{event_type}", "response": {{"id": "resp_warmup_{WARMUP_ID}", "object": "response", "created_at": {NOW}, "status": "{status}", "model": "{model}", "output": [], "usage": {{"input_tokens": 0, "output_tokens": 0, "total_tokens": 0}}}}}}"#
    ))
}

pub(super) fn frames(shared: &Shared) -> Vec<Json> {
    let recorder = lock(shared);
    recorder
        .frames
        .iter()
        .map(|frame| json(frame))
        .collect::<Vec<Json>>()
}

pub(super) fn calls(shared: &Shared) -> Vec<RecordedCall> {
    lock(shared).calls.clone()
}

pub(super) fn quotas(shared: &Shared) -> Vec<RecordedQuota> {
    lock(shared).quotas.clone()
}

pub(super) fn trace(shared: &Shared) -> Vec<String> {
    lock(shared).trace.clone()
}

pub(super) fn history(handler: &ManagedResponsesWebSocket<ScriptedSink>) -> Json {
    let stored = handler.session_history.iter().map(|(id, messages)| {
        let items = messages.iter().cloned().map(Json::Object).collect();
        (id.clone(), Json::List(items))
    });
    Json::Object(stored.collect())
}

#[derive(Clone, Default)]
struct CapturedLogs(Arc<Mutex<Vec<String>>>);

impl CapturedLogs {
    fn take(&self) -> Vec<String> {
        let mut messages = lock(&self.0);
        take(&mut *messages)
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

impl<Inner: Subscriber> Layer<Inner> for CapturedLogs {
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

pub(super) async fn capture<Output, Pending>(future: Pending) -> (Output, Vec<String>)
where
    Pending: Future<Output = Output> + Send,
{
    let logs = CapturedLogs::default();
    let subscriber = tracing_subscriber::registry().with(logs.clone());
    let output = future.with_subscriber(subscriber).await;
    (output, logs.take())
}
