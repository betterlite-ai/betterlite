#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::fmt::Debug;
use std::mem::take;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::error::ProviderError;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::passthrough::transformation::types::{
    PassthroughError, PassthroughLogging, PassthroughTarget, PassthroughUrlRequest, RelayCallType,
};
use betterlite_openai::passthrough::transformation::{
    BasePassthroughConfig, PassthroughStreamCollector,
};
use futures_util::future::BoxFuture;
use http::{HeaderMap, HeaderValue, Method};
use indexmap::IndexMap;
use tokio::sync::Notify;
use tokio_util::task::TaskTracker;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context as LayerContext, Layer, SubscriberExt as _};
use tracing_subscriber::registry;
use url::Url;

use super::async_lifecycle::AsyncPassthroughStream;
use super::async_passthrough_request::{PassthroughSender, PendingSend};
use super::error::PassthroughStreamError;
use super::logging::{SpendLogging, SyncFlushExecutor};
use super::response::{
    AsyncChunkSource, AsyncResponse, PendingRelay, SyncChunkSource, SyncResponse,
};
use super::spend_collection::{BoxedCollector, SpendCollection, SpendSink};
use super::sync_lifecycle::PassthroughStreamingResponse;

pub(crate) const MODEL: &str = "gpt-test";
pub(crate) const PROVIDER: &str = "openai";
pub(crate) const ENDPOINT: &str = "/v1/chat/completions";

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub(crate) struct FakeConfigError {
    pub(crate) message: String,
}

impl From<ProviderError> for FakeConfigError {
    fn from(error: ProviderError) -> Self {
        Self {
            message: error.message,
        }
    }
}

impl From<PassthroughError> for FakeConfigError {
    fn from(error: PassthroughError) -> Self {
        Self {
            message: error.to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Collected {
    pub(crate) model: String,
    pub(crate) custom_llm_provider: String,
    pub(crate) endpoint: String,
    pub(crate) lines: Vec<String>,
}

pub(crate) struct FakeConfig {
    pub(crate) streaming: bool,
    pub(crate) collected: Arc<Mutex<Vec<Collected>>>,
}

pub(crate) fn fake_config() -> FakeConfig {
    FakeConfig {
        streaming: true,
        collected: Arc::new(Mutex::new(Vec::new())),
    }
}

impl BasePassthroughConfig for FakeConfig {
    type Error = FakeConfigError;
    type Logged = String;

    fn is_streaming_request(&self, _endpoint: &str, _request_data: &Object) -> bool {
        self.streaming
    }

    fn get_complete_url(
        &self,
        request: &PassthroughUrlRequest<'_>,
    ) -> Result<(Url, String), Self::Error> {
        let base = request.api_base.unwrap_or("https://api.test");
        let url = self.format_url(request.target.endpoint, base, None)?;
        Ok((url, base.to_owned()))
    }

    fn handle_collected_chunks(
        &self,
        all_chunks: &[String],
        _logging_obj: &mut dyn PassthroughLogging,
        target: &PassthroughTarget<'_>,
    ) -> Option<Self::Logged> {
        self.collected
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Collected {
                model: target.model.to_owned(),
                custom_llm_provider: target.custom_llm_provider.to_owned(),
                endpoint: target.endpoint.to_owned(),
                lines: all_chunks.to_vec(),
            });
        Some(all_chunks.join(";"))
    }
}

pub(crate) struct NoopPassthroughLogging;

impl PassthroughLogging for NoopPassthroughLogging {
    fn set_call_type(&mut self, _call_type: RelayCallType) {}
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub(crate) struct FakeSinkError {
    pub(crate) message: String,
}

pub(crate) struct SinkState {
    pub(crate) chunks: Vec<Vec<u8>>,
    pub(crate) calls: usize,
    pub(crate) fail_at_call: Option<usize>,
}

pub(crate) struct RecordingSink {
    pub(crate) state: Arc<Mutex<SinkState>>,
}

impl SpendSink for RecordingSink {
    type Logged = String;
    type Failure = FakeSinkError;

    fn add(&mut self, chunk: &[u8]) -> Result<(), Self::Failure> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.calls = state.calls.saturating_add(1);
        if state.fail_at_call == Some(state.calls) {
            return Err(FakeSinkError {
                message: format!("collector add {} failed", state.calls),
            });
        }
        state.chunks.push(chunk.to_vec());
        drop(state);
        Ok(())
    }

    fn into_collector(self) -> BoxedCollector<String> {
        let chunks = self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .chunks
            .clone();
        Box::new(RecordingCollector { chunks })
    }
}

pub(crate) struct RecordingCollector {
    pub(crate) chunks: Vec<Vec<u8>>,
}

impl PassthroughStreamCollector for RecordingCollector {
    type Logged = String;

    fn add(&mut self, chunk: &[u8]) {
        self.chunks.push(chunk.to_vec());
    }

    fn build_logged_response(
        &self,
        _logging_obj: &mut dyn PassthroughLogging,
    ) -> Result<Option<Self::Logged>, PassthroughError> {
        let joined = self.chunks.concat();
        let text = String::from_utf8(joined).map_err(|error| PassthroughError::InvalidUtf8 {
            source: error.utf8_error(),
        })?;
        Ok(Some(text))
    }
}

pub(crate) fn collection() -> (SpendCollection<RecordingSink>, Arc<Mutex<SinkState>>) {
    let state = Arc::new(Mutex::new(SinkState {
        chunks: Vec::new(),
        calls: 0,
        fail_at_call: None,
    }));
    let sink = RecordingSink {
        state: Arc::clone(&state),
    };
    let collection = SpendCollection {
        sink: Some(sink),
        chunk_count: 0,
        failed: false,
    };
    (collection, state)
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub(crate) struct FakeFlushError {
    pub(crate) message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Flush {
    pub(crate) model: String,
    pub(crate) custom_llm_provider: String,
    pub(crate) endpoint: String,
    pub(crate) logged: String,
}

pub(crate) struct FakeLogging {
    pub(crate) model: String,
    pub(crate) custom_llm_provider: String,
    pub(crate) endpoint: String,
    pub(crate) flushes: Arc<Mutex<Vec<Flush>>>,
    pub(crate) fail: bool,
    pub(crate) gate: Option<Arc<Notify>>,
    pub(crate) started: Arc<Notify>,
}

impl FakeLogging {
    #[must_use]
    pub(crate) fn fixture() -> Self {
        Self {
            model: MODEL.to_owned(),
            custom_llm_provider: PROVIDER.to_owned(),
            endpoint: ENDPOINT.to_owned(),
            flushes: Arc::new(Mutex::new(Vec::new())),
            fail: false,
            gate: None,
            started: Arc::new(Notify::new()),
        }
    }

    #[must_use]
    pub(crate) fn failing(mut self) -> Self {
        self.fail = true;
        self
    }

    #[must_use]
    pub(crate) fn gated(mut self, gate: Arc<Notify>) -> Self {
        self.gate = Some(gate);
        self
    }

    pub(crate) fn flushed(&self) -> Vec<Flush> {
        self.flushes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(
        &self,
        collector: &dyn PassthroughStreamCollector<Logged = String>,
    ) -> Result<(), FakeFlushError> {
        let mut logging = NoopPassthroughLogging;
        let logged = collector
            .build_logged_response(&mut logging)
            .map_err(|error| FakeFlushError {
                message: error.to_string(),
            })?;
        self.flushes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Flush {
                model: self.model.clone(),
                custom_llm_provider: self.custom_llm_provider.clone(),
                endpoint: self.endpoint.clone(),
                logged: logged.unwrap_or_default(),
            });
        if self.fail {
            return Err(FakeFlushError {
                message: "the spend flush failed".to_owned(),
            });
        }
        Ok(())
    }
}

#[async_trait]
impl SpendLogging<String> for FakeLogging {
    type FlushError = FakeFlushError;

    fn spend_target(&self) -> PassthroughTarget<'_> {
        PassthroughTarget {
            model: &self.model,
            custom_llm_provider: &self.custom_llm_provider,
            endpoint: &self.endpoint,
        }
    }

    async fn async_flush_collected_chunks(
        &self,
        collector: BoxedCollector<String>,
    ) -> Result<(), Self::FlushError> {
        if let Some(gate) = &self.gate {
            self.started.notify_one();
            gate.notified().await;
        }
        self.record(collector.as_ref())
    }

    fn flush_collected_chunks(
        &self,
        collector: BoxedCollector<String>,
    ) -> Result<(), Self::FlushError> {
        self.record(collector.as_ref())
    }
}

#[derive(Clone, Debug, thiserror::Error)]
#[error("the executor is shut down")]
pub(crate) struct FakeScheduleError;

pub(crate) type FlushJob = Box<dyn FnOnce() + Send + 'static>;

#[derive(Clone)]
pub(crate) struct RecordingExecutor {
    pub(crate) jobs: Arc<Mutex<Vec<FlushJob>>>,
    pub(crate) reject: bool,
}

pub(crate) fn inline_executor() -> RecordingExecutor {
    RecordingExecutor {
        jobs: Arc::new(Mutex::new(Vec::new())),
        reject: false,
    }
}

impl RecordingExecutor {
    pub(crate) fn run_jobs(&self) {
        let mut guard = self.jobs.lock().unwrap_or_else(PoisonError::into_inner);
        let jobs = take(&mut *guard);
        drop(guard);
        for job in jobs {
            job();
        }
    }

    pub(crate) fn scheduled(&self) -> usize {
        self.jobs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

impl SyncFlushExecutor for RecordingExecutor {
    type ScheduleError = FakeScheduleError;

    fn submit(&self, job: FlushJob) -> Result<(), Self::ScheduleError> {
        if self.reject {
            return Err(FakeScheduleError);
        }
        self.jobs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(job);
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) enum Step {
    Chunk(Vec<u8>),
    Fail,
}

#[derive(Clone)]
pub(crate) struct Script {
    pub(crate) steps: VecDeque<Step>,
    pub(crate) body: Option<Vec<u8>>,
    pub(crate) body_fails: bool,
    pub(crate) close_fails: bool,
}

pub(crate) fn script(steps: Vec<Step>) -> Script {
    Script {
        steps: VecDeque::from(steps),
        body: None,
        body_fails: false,
        close_fails: false,
    }
}

pub(crate) fn chunk_steps(parts: &[&str]) -> Vec<Step> {
    parts
        .iter()
        .map(|part| Step::Chunk(part.as_bytes().to_vec()))
        .collect()
}

#[derive(Default)]
pub(crate) struct SourceLog {
    pub(crate) closed: usize,
    pub(crate) body_reads: usize,
}

pub(crate) struct ScriptedSource {
    pub(crate) script: Script,
    pub(crate) log: Arc<Mutex<SourceLog>>,
}

impl ScriptedSource {
    fn take_chunk(&mut self) -> Result<Option<Vec<u8>>, HttpError> {
        match self.script.steps.pop_front() {
            Some(Step::Chunk(bytes)) => Ok(Some(bytes)),
            Some(Step::Fail) => Err(HttpError::Timeout),
            None => Ok(None),
        }
    }

    fn take_body(&mut self) -> Result<Vec<u8>, HttpError> {
        self.note_body_read();
        if self.script.body_fails {
            return Err(HttpError::Timeout);
        }
        Ok(self.script.body.take().unwrap_or_default())
    }

    fn finish_close(&mut self) -> Result<(), HttpError> {
        self.note_close();
        self.script.steps.clear();
        if self.script.close_fails {
            return Err(HttpError::Timeout);
        }
        Ok(())
    }

    fn note_body_read(&self) {
        let mut log = self.log.lock().unwrap_or_else(PoisonError::into_inner);
        log.body_reads = log.body_reads.saturating_add(1);
    }

    fn note_close(&self) {
        let mut log = self.log.lock().unwrap_or_else(PoisonError::into_inner);
        log.closed = log.closed.saturating_add(1);
    }
}

impl AsyncChunkSource for ScriptedSource {
    fn next_chunk(&mut self) -> BoxFuture<'_, Result<Option<Vec<u8>>, HttpError>> {
        Box::pin(async move { self.take_chunk() })
    }

    fn read_body(&mut self) -> BoxFuture<'_, Result<Vec<u8>, HttpError>> {
        Box::pin(async move { self.take_body() })
    }

    fn close(&mut self) -> BoxFuture<'_, Result<(), HttpError>> {
        Box::pin(async move { self.finish_close() })
    }
}

impl SyncChunkSource for ScriptedSource {
    fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, HttpError> {
        self.take_chunk()
    }

    fn close(&mut self) -> Result<(), HttpError> {
        self.finish_close()
    }
}

pub(crate) struct ReplyFixture {
    pub(crate) status_code: u16,
    pub(crate) headers: HeaderMap,
    pub(crate) script: Script,
    pub(crate) log: Arc<Mutex<SourceLog>>,
}

impl ReplyFixture {
    #[must_use]
    pub(crate) fn ok(script: Script) -> Self {
        let mut headers = HeaderMap::new();
        headers.insert("x-upstream", HeaderValue::from_static("yes"));
        Self {
            status_code: 200,
            headers,
            script,
            log: Arc::new(Mutex::new(SourceLog::default())),
        }
    }

    #[must_use]
    pub(crate) fn with_status(mut self, status_code: u16) -> Self {
        self.status_code = status_code;
        self
    }

    #[must_use]
    pub(crate) fn with_body(mut self, body: &str) -> Self {
        self.script.body = Some(body.as_bytes().to_vec());
        self
    }

    #[must_use]
    pub(crate) fn failing_body(mut self) -> Self {
        self.script.body_fails = true;
        self
    }

    #[must_use]
    pub(crate) fn failing_close(mut self) -> Self {
        self.script.close_fails = true;
        self
    }

    pub(crate) fn log(&self) -> Arc<Mutex<SourceLog>> {
        Arc::clone(&self.log)
    }

    pub(crate) fn into_pending(self) -> PendingRelay {
        let Self {
            status_code,
            headers,
            script,
            log,
        } = self;
        Box::pin(async move {
            Ok(AsyncResponse {
                status_code,
                headers,
                source: Box::new(ScriptedSource { script, log }),
            })
        })
    }

    pub(crate) fn into_sync(self) -> SyncResponse {
        let Self {
            status_code,
            headers,
            script,
            log,
        } = self;
        SyncResponse {
            status_code,
            headers,
            source: Box::new(ScriptedSource { script, log }),
        }
    }
}

#[must_use]
pub(crate) fn failed_send() -> PendingRelay {
    Box::pin(async {
        Err(PassthroughStreamError::Send(Box::new(
            TransportError::Closed,
        )))
    })
}

#[must_use]
pub(crate) fn flagged_pending(reply: ReplyFixture, awaited: Arc<AtomicBool>) -> PendingRelay {
    let pending = reply.into_pending();
    Box::pin(async move {
        awaited.store(true, Ordering::SeqCst);
        pending.await
    })
}

pub(crate) struct Fixture {
    pub(crate) stream: AsyncPassthroughStream<RecordingSink, FakeLogging>,
    pub(crate) sink: Arc<Mutex<SinkState>>,
    pub(crate) logging: Arc<FakeLogging>,
    pub(crate) source: Arc<Mutex<SourceLog>>,
    pub(crate) tracker: TaskTracker,
}

impl Fixture {
    #[must_use]
    pub(crate) fn awaiting(reply: ReplyFixture) -> Self {
        let source = reply.log();
        Self::from_pending(reply.into_pending(), source)
    }

    #[must_use]
    pub(crate) fn from_pending(pending: PendingRelay, source: Arc<Mutex<SourceLog>>) -> Self {
        let (spend, sink) = collection();
        let logging = Arc::new(FakeLogging::fixture());
        let tracker = TaskTracker::new();
        let stream = AsyncPassthroughStream {
            initialized: false,
            status_code: 0,
            headers: HeaderMap::new(),
            pending: Some(pending),
            source: None,
            spend,
            logging: Arc::clone(&logging),
            background_tasks: tracker.clone(),
            flush_scheduled: false,
            hidden_params: IndexMap::new(),
        };
        Self {
            stream,
            sink,
            logging,
            source,
            tracker,
        }
    }

    pub(crate) async fn ready(reply: ReplyFixture) -> Self {
        let mut fixture = Self::awaiting(reply);
        fixture
            .stream
            .init()
            .await
            .expect("the success reply initializes");
        fixture
    }

    pub(crate) fn settled(&self) -> BoxFuture<'_, ()> {
        let tracker = self.tracker.clone();
        Box::pin(async move {
            tracker.close();
            tracker.wait().await;
        })
    }
}

pub(crate) struct SyncFixture {
    pub(crate) stream: PassthroughStreamingResponse<RecordingSink, FakeLogging, RecordingExecutor>,
    pub(crate) sink: Arc<Mutex<SinkState>>,
    pub(crate) logging: Arc<FakeLogging>,
    pub(crate) source: Arc<Mutex<SourceLog>>,
    pub(crate) executor: RecordingExecutor,
}

impl SyncFixture {
    #[must_use]
    pub(crate) fn new(reply: ReplyFixture, executor: RecordingExecutor) -> Self {
        let source = reply.log();
        let (spend, sink) = collection();
        let logging = Arc::new(FakeLogging::fixture());
        let response = reply.into_sync();
        let stream = PassthroughStreamingResponse {
            status_code: response.status_code,
            headers: response.headers,
            source: response.source,
            spend,
            logging: Arc::clone(&logging),
            executor: executor.clone(),
            flush_scheduled: false,
        };
        Self {
            stream,
            sink,
            logging,
            source,
            executor,
        }
    }
}

pub(crate) struct FakeSender {
    pub(crate) reply: ReplyFixture,
    pub(crate) immediate: bool,
    pub(crate) send_fails: bool,
}

impl FakeSender {
    #[must_use]
    pub(crate) fn replying(reply: ReplyFixture) -> Self {
        Self {
            reply,
            immediate: false,
            send_fails: false,
        }
    }

    #[must_use]
    pub(crate) fn immediate() -> Self {
        Self {
            reply: ReplyFixture::ok(script(Vec::new())),
            immediate: true,
            send_fails: false,
        }
    }

    #[must_use]
    pub(crate) fn failing() -> Self {
        Self {
            reply: ReplyFixture::ok(script(Vec::new())),
            immediate: false,
            send_fails: true,
        }
    }

    pub(crate) fn log(&self) -> Arc<Mutex<SourceLog>> {
        Arc::clone(&self.reply.log)
    }
}

impl PassthroughSender for FakeSender {
    fn send_passthrough(&self, _request: OutboundRequest) -> PendingSend {
        if self.immediate {
            return PendingSend::Immediate;
        }
        if self.send_fails {
            return PendingSend::Coroutine(Box::pin(async {
                Err(PassthroughStreamError::Send(Box::new(
                    TransportError::Closed,
                )))
            }));
        }
        let status_code = self.reply.status_code;
        let headers = self.reply.headers.clone();
        let source = ScriptedSource {
            script: self.reply.script.clone(),
            log: Arc::clone(&self.reply.log),
        };
        PendingSend::Coroutine(Box::pin(async move {
            Ok(AsyncResponse {
                status_code,
                headers,
                source: Box::new(source),
            })
        }))
    }
}

pub(crate) fn proxy_client(body_bytes: usize) -> ProxiedClient {
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &environment).unwrap());
    let mut client = ProxiedClient::outbound(&policy).unwrap();
    client.limits.body_bytes = body_bytes;
    client
}

pub(crate) fn outbound_request(url: &str) -> OutboundRequest {
    OutboundRequest {
        method: Method::POST,
        url: Url::parse(url).unwrap(),
        query: Vec::new(),
        headers: HeaderMap::new(),
        body: RequestBody::Empty,
        deadline: Duration::from_secs(5),
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CapturedEvent {
    pub(crate) level: Level,
    pub(crate) message: String,
    pub(crate) fields: Vec<String>,
}

#[derive(Clone, Default)]
pub(crate) struct Capture(Arc<Mutex<Vec<CapturedEvent>>>);

impl Capture {
    pub(crate) fn events(&self) -> Vec<CapturedEvent> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn messages(&self) -> Vec<String> {
        self.events()
            .into_iter()
            .map(|event| event.message)
            .collect()
    }
}

#[derive(Default)]
struct EventVisitor {
    message: String,
    fields: Vec<String>,
}

impl Visit for EventVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        let text = format!("{value:?}");
        self.capture(field, text);
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.capture(field, value.to_owned());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.capture(field, value.to_string());
    }
}

impl EventVisitor {
    fn capture(&mut self, field: &Field, text: String) {
        if field.name() == "message" {
            self.message = text;
            return;
        }
        self.fields.push(format!("{}={text}", field.name()));
    }
}

impl<Inner: Subscriber> Layer<Inner> for Capture {
    fn on_event(&self, event: &Event<'_>, _context: LayerContext<'_, Inner>) {
        let mut visitor = EventVisitor::default();
        event.record(&mut visitor);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(CapturedEvent {
                level: *event.metadata().level(),
                message: visitor.message,
                fields: visitor.fields,
            });
    }
}

pub(crate) fn capture() -> (Capture, DefaultGuard) {
    let captured = Capture::default();
    let guard = set_default(registry().with(captured.clone()));
    (captured, guard)
}
