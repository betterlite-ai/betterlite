#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::io::{self, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_config::tls::Verification;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::user_url::error::UserUrlError;
use betterlite_http::user_url::validate::{Resolve, UrlValidator};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use crate::callback_logger::event::LoggerEvent;
use crate::fields::payload::StandardLoggingPayload;
use crate::fields::status::{GuardrailStatus, LoggingStatus, StatusFields};
use crate::finops::pointfive::upload::url_check::{UploadUrlCheck, ValidatedUpload};
use crate::posthog_logging::batch::QueuedEvent;
use crate::posthog_logging::delivery::{PostFailure, PostRequest, Transport};
use crate::posthog_logging::event::{IdGenerator, create_event_payload};
use crate::posthog_logging::logger::{PostHogDeps, PostHogLogger};
use crate::posthog_logging::settings::{PostHogSettings, resolve};

pub(super) const API_KEY: &str = "phk-test";
pub(super) const API_URL: &str = "https://us.i.posthog.com";

#[derive(Clone, Copy, Debug)]
pub(super) struct Reply {
    pub status: u16,
    pub network: bool,
}

impl Reply {
    pub(super) const fn status(status: u16) -> Self {
        Self {
            status,
            network: false,
        }
    }

    pub(super) const fn network() -> Self {
        Self {
            status: 0,
            network: true,
        }
    }
}

pub(super) struct RecordedRequest {
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub body: Json,
}

pub(super) struct ScriptedTransport {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<RecordedRequest>>,
}

impl ScriptedTransport {
    pub(super) fn new(replies: &[Reply]) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies.iter().copied().collect()),
            requests: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn requests(&self) -> Vec<RecordedRequest> {
        let recorded = self.requests.lock().unwrap_or_else(PoisonError::into_inner);
        recorded
            .iter()
            .map(|request| RecordedRequest {
                url: request.url.clone(),
                headers: request.headers.clone(),
                body: request.body.clone(),
            })
            .collect()
    }

    pub(super) fn urls(&self) -> Vec<String> {
        self.requests()
            .into_iter()
            .map(|request| request.url)
            .collect()
    }
}

#[async_trait]
impl Transport for ScriptedTransport {
    async fn post(&self, request: PostRequest) -> Result<u16, PostFailure> {
        let body: Json = serde_json::from_slice(&request.body).unwrap_or(Json::Null);
        let headers = request
            .headers
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_ascii_lowercase(),
                    value.to_str().unwrap_or_default().to_owned(),
                )
            })
            .collect();
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedRequest {
                url: request.url,
                headers,
                body,
            });
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(Reply::status(200));
        if reply.network {
            return Err(PostFailure::Request(TransportError::Rejected(
                HttpError::Timeout,
            )));
        }
        Ok(reply.status)
    }
}

pub(super) fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn settings(pairs: &[(&str, &str)]) -> PostHogSettings {
    resolve(&environment(pairs)).expect("the settings resolve")
}

pub(super) fn default_settings() -> PostHogSettings {
    settings(&[("POSTHOG_API_KEY", API_KEY), ("POSTHOG_API_URL", API_URL)])
}

pub(super) fn fixed_id() -> String {
    "generated-id".to_owned()
}

#[derive(Clone, Copy, Debug)]
pub(super) struct AllowAllUrlCheck;

#[async_trait]
impl UploadUrlCheck for AllowAllUrlCheck {
    async fn check(&self, url: &str) -> Result<ValidatedUpload, UserUrlError> {
        Ok(ValidatedUpload {
            url: url.to_owned(),
            host_header: None,
        })
    }
}

struct TestResolver;

#[async_trait]
impl Resolve for TestResolver {
    async fn resolve(&self, host: &str, _port: u16) -> io::Result<Vec<SocketAddr>> {
        let address = if host == "private.example" {
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        } else {
            IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))
        };
        Ok(vec![SocketAddr::new(address, 443)])
    }
}

pub(super) fn validating_check() -> Arc<dyn UploadUrlCheck> {
    Arc::new(UrlValidator::new(TestResolver, &[], &Verification::Enabled))
}

fn build(
    settings: PostHogSettings,
    transport: &Arc<ScriptedTransport>,
    url_check: Arc<dyn UploadUrlCheck>,
    ids: IdGenerator,
    shutdown: Option<CancellationToken>,
) -> (Arc<PostHogLogger>, TaskTracker) {
    let tasks = TaskTracker::new();
    let concrete = Arc::clone(transport);
    let target: Arc<dyn Transport> = concrete;
    let logger = PostHogLogger::new(
        settings,
        PostHogDeps {
            transport: target,
            url_check,
            ids,
            tasks: tasks.clone(),
            shutdown,
        },
    );
    (logger, tasks)
}

pub(super) fn logger_with(
    settings: PostHogSettings,
    transport: &Arc<ScriptedTransport>,
    ids: IdGenerator,
) -> (Arc<PostHogLogger>, TaskTracker) {
    build(settings, transport, Arc::new(AllowAllUrlCheck), ids, None)
}

pub(super) fn logger_with_check(
    settings: PostHogSettings,
    transport: &Arc<ScriptedTransport>,
    url_check: Arc<dyn UploadUrlCheck>,
    ids: IdGenerator,
) -> (Arc<PostHogLogger>, TaskTracker) {
    build(settings, transport, url_check, ids, None)
}

pub(super) fn logger_with_shutdown(
    settings: PostHogSettings,
    transport: &Arc<ScriptedTransport>,
    ids: IdGenerator,
) -> (Arc<PostHogLogger>, TaskTracker, CancellationToken) {
    let shutdown = CancellationToken::new();
    let (logger, tasks) = build(
        settings,
        transport,
        Arc::new(AllowAllUrlCheck),
        ids,
        Some(shutdown.clone()),
    );
    (logger, tasks, shutdown)
}

// serde fills every optional field, so the fixture names only what the projection reads
const PAYLOAD: &str = r#"
{
  "id": "span-1",
  "trace_id": "trace-1",
  "session_id": "session-1",
  "call_type": "completion",
  "response_cost": 0.5,
  "status": "success",
  "status_fields": {"llm_api_status": "success", "guardrail_status": "not_run"},
  "custom_llm_provider": "openai",
  "total_tokens": 15,
  "prompt_tokens": 10,
  "completion_tokens": 5,
  "startTime": 1700000000,
  "endTime": 1700000001,
  "completionStartTime": 1700000000.5,
  "response_time": 1,
  "model_map_information": {},
  "model": "gpt-4o",
  "api_base": "https://api.openai.com",
  "metadata": {},
  "saved_cache_cost": 0,
  "request_tags": [],
  "model_parameters": {},
  "hidden_params": {},
  "messages": [{"role": "user", "content": "hi"}],
  "response": {"choices": []}
}
"#;

pub(super) fn payload() -> StandardLoggingPayload {
    serde_json::from_str(PAYLOAD).expect("the fixture payload parses")
}

pub(super) fn failed_payload() -> StandardLoggingPayload {
    let mut payload = payload();
    payload.status = LoggingStatus::Failure;
    payload.status_fields = StatusFields {
        llm_api_status: LoggingStatus::Failure,
        guardrail_status: GuardrailStatus::NotRun,
    };
    payload.error_str = Some("upstream refused".to_owned());
    payload
}

pub(super) fn event(payload: StandardLoggingPayload) -> LoggerEvent {
    event_with(payload, Json::Null)
}

pub(super) fn event_with(payload: StandardLoggingPayload, dynamic_params: Json) -> LoggerEvent {
    LoggerEvent {
        payload: Arc::new(payload),
        dynamic_params: Arc::new(dynamic_params),
        litellm_metadata: Arc::new(IndexMap::new()),
        stream: false,
        guardrail_snapshot: None,
    }
}

pub(super) fn object(pairs: &[(&str, Json)]) -> Json {
    Json::Object(
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect::<IndexMap<String, Json>>(),
    )
}

pub(super) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(super) fn number(value: i64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

pub(super) fn field<'body>(body: &'body Json, name: &str) -> &'body Json {
    match body {
        Json::Object(fields) => fields.get(name).expect("the field is present"),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("the body is not an object")
        }
    }
}

pub(super) fn list_len(value: &Json) -> usize {
    match value {
        Json::List(items) => items.len(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => 0,
    }
}

pub(super) fn items(value: &Json) -> &[Json] {
    match value {
        Json::List(items) => items,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => &[],
    }
}

pub(super) fn queued(payload: &StandardLoggingPayload) -> QueuedEvent {
    queued_for(API_KEY, API_URL, payload)
}

pub(super) fn queued_for(
    api_key: &str,
    api_url: &str,
    payload: &StandardLoggingPayload,
) -> QueuedEvent {
    let event = create_event_payload(payload, fixed_id).expect("the event builds");
    QueuedEvent {
        event,
        api_key: SecretString::from(api_key),
        api_url: api_url.to_owned(),
        request_url: false,
    }
}

pub(super) async fn settle(tasks: &TaskTracker) {
    tasks.close();
    tasks.wait().await;
}

pub(super) async fn stop(logger: &PostHogLogger, tasks: &TaskTracker) {
    logger.stopped.cancel();
    tasks.close();
    tasks.wait().await;
}

pub(super) fn dynamic(pairs: &[(&str, &str)]) -> Json {
    Json::Object(
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), Json::Text((*value).to_owned())))
            .collect::<IndexMap<String, Json>>(),
    )
}

#[derive(Clone, Default)]
pub(super) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub(super) fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8(bytes.clone()).expect("the log output is UTF-8")
    }
}

impl Write for LogBuffer {
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

    fn make_writer(&self) -> Self::Writer {
        self.clone()
    }
}

pub(super) fn capture_logs() -> (DefaultGuard, LogBuffer) {
    let buffer = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    (set_default(subscriber), buffer)
}
