use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::io::{Result as IoResult, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_models::decisions::Json;
use chrono::{DateTime, TimeZone as _, Utc};
use http::HeaderMap;
use indexmap::IndexMap;
use tokio_util::task::TaskTracker;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use crate::http_analytics_loggers::delivery::{Delivery, PostRequest, PostResponse};
use crate::opik::delivery::callback::Call;
use crate::opik::delivery::logger::{LoggerDependencies, OpikLogger};
use crate::opik::delivery::native::{NativeClientError, NativeClientSource, NativeOpikClient};
use crate::opik::delivery::record::{SpanRecord, TraceRecord};
use crate::opik::delivery::settings::{LoggerOptions, OpikSettingsError};

pub(super) const TRACE_URL: &str = "https://www.comet.com/opik/api/v1/private/traces/batch";
pub(super) const SPAN_URL: &str = "https://www.comet.com/opik/api/v1/private/spans/batch";

#[derive(Clone, Default)]
pub(super) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub(super) fn contents(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl Write for LogBuffer {
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

impl<'writer> MakeWriter<'writer> for LogBuffer {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

pub(super) fn capture_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    let guard = set_default(subscriber);
    (buffer, guard)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Reply {
    Status(u16),
    Transport,
}

#[derive(Clone, Debug)]
pub(super) struct RecordedRequest {
    pub(super) url: String,
    pub(super) headers: HeaderMap,
    pub(super) body: Vec<u8>,
}

impl RecordedRequest {
    pub(super) fn body_json(&self) -> Json {
        serde_json::from_slice(&self.body).expect("the recorded body is JSON")
    }

    pub(super) fn batch(&self, field: &str) -> Vec<IndexMap<String, Json>> {
        let Json::Object(batch) = self.body_json() else {
            panic!("the Opik batch body is an object")
        };
        let Some(Json::List(entries)) = batch.get(field) else {
            panic!("the Opik batch body has a {field} list")
        };
        entries
            .iter()
            .map(|entry| match entry {
                Json::Object(values) => values.clone(),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                    panic!("an Opik batch entry is an object")
                }
            })
            .collect()
    }
}

#[derive(Clone, Default)]
pub(super) struct RecordingDelivery {
    state: Arc<DeliveryState>,
}

#[derive(Default)]
struct DeliveryState {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<RecordedRequest>>,
}

impl RecordingDelivery {
    pub(super) fn scripted<R>(replies: R) -> Self
    where
        R: IntoIterator<Item = Reply>,
    {
        let delivery = Self::default();
        delivery.state.replies.lock().unwrap().extend(replies);
        delivery
    }

    pub(super) fn requests(&self) -> Vec<RecordedRequest> {
        self.state.requests.lock().unwrap().clone()
    }

    pub(super) fn urls(&self) -> Vec<String> {
        self.requests()
            .into_iter()
            .map(|request| request.url)
            .collect()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the scripted Opik delivery failed")]
pub(super) struct ScriptedFailure;

impl Delivery for RecordingDelivery {
    type Error = ScriptedFailure;

    fn post(
        &self,
        request: PostRequest,
    ) -> impl Future<Output = Result<PostResponse, Self::Error>> + Send {
        let reply = self
            .state
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(Reply::Status(204));
        self.state
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedRequest {
                url: request.url.to_string(),
                headers: request.headers,
                body: request.body,
            });
        async move {
            match reply {
                Reply::Status(status) => Ok(PostResponse {
                    status,
                    body: b"opik response".to_vec(),
                }),
                Reply::Transport => Err(ScriptedFailure),
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NativeCallKind {
    Trace,
    Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum NativeCall {
    Trace(TraceRecord),
    Span(SpanRecord),
}

pub(super) struct RecordingNative {
    calls: Mutex<Vec<NativeCall>>,
    fail: Option<NativeCallKind>,
}

impl RecordingNative {
    pub(super) fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            fail: None,
        }
    }

    pub(super) fn failing(kind: NativeCallKind) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            fail: Some(kind),
        }
    }

    pub(super) fn calls(&self) -> Vec<NativeCall> {
        self.calls.lock().unwrap().clone()
    }
}

impl NativeOpikClient for RecordingNative {
    fn trace(&self, trace: &TraceRecord) -> Result<(), NativeClientError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(NativeCall::Trace(trace.clone()));
        if self.fail == Some(NativeCallKind::Trace) {
            return Err(NativeClientError {
                message: "the native trace call failed".to_owned(),
            });
        }
        Ok(())
    }

    fn span(&self, span: &SpanRecord) -> Result<(), NativeClientError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(NativeCall::Span(span.clone()));
        if self.fail == Some(NativeCallKind::Span) {
            return Err(NativeClientError {
                message: "the native span call failed".to_owned(),
            });
        }
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct FixedSource {
    client: Option<Arc<dyn NativeOpikClient>>,
    calls: AtomicUsize,
}

impl FixedSource {
    pub(super) fn with_client(client: Arc<RecordingNative>) -> Self {
        let dynamic: Arc<dyn NativeOpikClient> = client;
        Self {
            client: Some(dynamic),
            calls: AtomicUsize::new(0),
        }
    }

    pub(super) fn installed_calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl NativeClientSource for FixedSource {
    fn get_client_cached(&self) -> Option<Arc<dyn NativeOpikClient>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.client.clone()
    }
}

pub(super) struct Fixture<'values> {
    pub(super) delivery: RecordingDelivery,
    pub(super) source: FixedSource,
    pub(super) options: LoggerOptions<'values>,
    pub(super) environment: BTreeMap<String, String>,
    pub(super) config_file: Option<&'values str>,
}

impl Fixture<'_> {
    pub(super) fn new() -> Self {
        Self {
            delivery: RecordingDelivery::default(),
            source: FixedSource::default(),
            options: LoggerOptions::default(),
            environment: BTreeMap::new(),
            config_file: None,
        }
    }

    pub(super) fn scripted<R>(replies: R) -> Self
    where
        R: IntoIterator<Item = Reply>,
    {
        let mut fixture = Self::new();
        fixture.delivery = RecordingDelivery::scripted(replies);
        fixture
    }

    pub(super) fn try_build(
        &self,
        tasks: Option<&TaskTracker>,
    ) -> Result<Arc<OpikLogger<RecordingDelivery>>, OpikSettingsError> {
        OpikLogger::build(
            &self.options,
            &self.environment,
            self.config_file,
            LoggerDependencies {
                delivery: self.delivery.clone(),
                native: &self.source,
                tasks,
            },
        )
    }

    pub(super) fn build(&self, tasks: Option<&TaskTracker>) -> Arc<OpikLogger<RecordingDelivery>> {
        self.try_build(tasks)
            .expect("the fixture's Opik settings resolve")
    }
}

pub(super) fn object(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).expect("the fixture JSON is valid")
}

pub(super) fn millis(value: i64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(value).unwrap()
}

pub(super) fn sample_kwargs() -> IndexMap<String, Json> {
    object(
        r#"{
            "custom_llm_provider": "openai",
            "response_cost": 0.25,
            "litellm_params": {"metadata": {"opik": {"thread_id": "thread-1"}}},
            "standard_logging_object": {
                "messages": [{"role": "user", "content": "hi"}],
                "response": {"answer": "ok"},
                "model": "gpt-4o",
                "status": "success"
            }
        }"#,
    )
}

pub(super) fn minimal_kwargs() -> IndexMap<String, Json> {
    object(
        r#"{
            "standard_logging_object": {
                "messages": [{"role": "user", "content": "hi"}],
                "response": {"answer": "ok"},
                "model": "gpt-4o",
                "status": "success"
            }
        }"#,
    )
}

pub(super) fn sample_response() -> IndexMap<String, Json> {
    object(
        r#"{"model":"gpt-4o","object":"chat.completion","created":7,
            "usage":{"prompt_tokens":3,"completion_tokens":4,"total_tokens":7}}"#,
    )
}

pub(super) fn call<'event>(
    kwargs: &'event IndexMap<String, Json>,
    response: &'event IndexMap<String, Json>,
) -> Call<'event> {
    Call {
        kwargs,
        response,
        start_time: millis(1_000),
        end_time: millis(2_000),
    }
}
