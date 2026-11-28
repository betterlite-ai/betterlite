use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::base_builder;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_openai::files::records::{BinaryContent, FileContentRequest};
use betterlite_openai::files::stream::FileContentStreamingResult;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use futures_util::stream::{BoxStream, Stream, StreamExt as _, poll_fn};
use http::HeaderMap as HttpHeaders;
use http::header::HeaderValue;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;
use tokio::sync::mpsc::{Receiver, Sender, channel};
use url::Url;

use crate::api::file_http_content::config::{
    BodyStream, BufferedResponse, ContentParams, FileContentConfig, Headers, QueryParams,
    StreamContext,
};
use crate::api::file_http_content::error::ContentError;
use crate::api::file_http_content::service::buffered::retrieve_file_content;
use crate::api::file_http_content::service::streaming::retrieve_content_streaming;
use crate::api::file_http_content::service::{
    BufferedReply, ClientSource, ContentDeps, ContentRequest, ContentTransport, LogContext,
    PreparedContent, RetrieveCall, StreamedReply, prepare,
};
use crate::api::vector_store_http_write::error::{
    BaseLlmException, ClientError, HeaderMap as ErrorHeaders,
};

#[derive(Clone, Default)]
pub(crate) struct Events(Arc<Mutex<Vec<String>>>);

impl Events {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn push(&self, name: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(name.to_owned());
    }

    pub(crate) fn recorded(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub(crate) struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

pub(crate) struct Fixture {
    redactor: Redactor,
    sink: Sink,
    pub(crate) state: LoggingState,
    now: DateTime<FixedOffset>,
}

impl Fixture {
    pub(crate) fn new(call_type: &str) -> Self {
        let init: LoggingInit = serde_json::from_str(&format!(
            r#"{{"model":"file-content","call_type":"{call_type}","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}}"#
        ))
        .expect("the logging fixture is valid");
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
        let now =
            DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").expect("the clock parses");
        let state = LoggingState::new(
            init,
            CorrelationSetup {
                context: &mut CorrelationContext::default(),
                settings: CorrelationSettings::default(),
            },
            &redactor,
            || String::from("generated"),
            Duration::default,
        )
        .expect("the logging state builds");
        Self {
            redactor,
            sink: Sink,
            state,
            now,
        }
    }

    pub(crate) fn prepare<'deps, C, S>(
        &mut self,
        deps: &'deps ContentDeps<'deps, C, S>,
        call: &RetrieveCall<'_>,
        events: &Events,
    ) -> Result<PreparedContent<'deps, C, S>, ContentError>
    where
        C: FileContentConfig,
        S: ClientSource,
    {
        let logger = |_details: &Json| -> Result<(), PrePostError> {
            events.push("log");
            Ok(())
        };
        let env = PrePostEnv {
            debug: DebugSettings::default(),
            globals: GlobalFlags::default(),
            decision: Decision {
                content: false,
                key_info: false,
            },
            redactor: &self.redactor,
            log: &self.sink,
            input_callbacks: &[],
            sentry: None,
            logger_fn: Some(&logger),
            now: self.now,
        };
        prepare(
            deps,
            call,
            &mut LogContext {
                state: &mut self.state,
                env: &env,
            },
        )
    }

    pub(crate) fn logged(&self, name: &str) -> Option<&Json> {
        let Json::Object(details) = self.state.model_call_details.get("additional_args")? else {
            return None;
        };
        details.get(name)
    }

    pub(crate) fn input(&self) -> Option<&Json> {
        self.state.model_call_details.get("input")
    }

    pub(crate) async fn fetch<'deps, C, S>(
        &mut self,
        deps: &'deps ContentDeps<'deps, C, S>,
        call: &RetrieveCall<'_>,
        events: &Events,
    ) -> Result<BinaryContent, ContentError>
    where
        C: FileContentConfig,
        S: ClientSource<Client: ContentTransport>,
    {
        let prepared = self.prepare(deps, call, events)?;
        retrieve_file_content(prepared, &mut self.state).await
    }
}

pub(crate) enum Answer {
    Buffered(u16, HttpHeaders, Vec<u8>),
    Stream(u16, HttpHeaders, BodyStream),
    Failure(TransportError),
}

// The wire answers one shape: a send that meets the other shape reads as a closed body
impl TryFrom<(Url, Answer)> for BufferedReply {
    type Error = TransportError;

    fn try_from((url, answer): (Url, Answer)) -> Result<Self, Self::Error> {
        match answer {
            Answer::Buffered(status, headers, body) => Ok(Self::new(status, url, headers, body)),
            Answer::Failure(error) => Err(error),
            Answer::Stream(..) => Err(TransportError::Closed),
        }
    }
}

impl TryFrom<(Url, Answer)> for StreamedReply {
    type Error = TransportError;

    fn try_from((url, answer): (Url, Answer)) -> Result<Self, Self::Error> {
        match answer {
            Answer::Stream(status, headers, body) => Ok(Self::new(status, url, headers, body)),
            Answer::Failure(error) => Err(error),
            Answer::Buffered(..) => Err(TransportError::Closed),
        }
    }
}

#[derive(Clone)]
pub(crate) struct Wire {
    answer: Arc<Mutex<Option<Answer>>>,
    requests: Arc<Mutex<Vec<ContentRequest>>>,
    events: Events,
}

impl Wire {
    pub(crate) fn answering(answer: Answer, events: &Events) -> Self {
        Self {
            answer: Arc::new(Mutex::new(Some(answer))),
            requests: Arc::default(),
            events: events.clone(),
        }
    }

    pub(crate) fn requests(&self) -> MutexGuard<'_, Vec<ContentRequest>> {
        self.requests.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn take(&self) -> Answer {
        self.answer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .expect("the wire answers once")
    }

    // every call records the request it carried and the URL it went to before the answer is taken
    fn record(&self, request: ContentRequest) -> (Url, Answer) {
        self.events.push("send");
        let answered = request.url.clone();
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        (answered, self.take())
    }
}

impl ContentTransport for Wire {
    fn send(
        &self,
        request: ContentRequest,
    ) -> impl Future<Output = Result<BufferedReply, TransportError>> + Send {
        let (answered, answer) = self.record(request);
        ready(BufferedReply::try_from((answered, answer)))
    }

    fn send_stream(
        &self,
        request: ContentRequest,
    ) -> impl Future<Output = Result<StreamedReply, TransportError>> + Send {
        let (answered, answer) = self.record(request);
        ready(StreamedReply::try_from((answered, answer)))
    }
}

pub(crate) struct Source {
    wire: Wire,
}

impl Source {
    pub(crate) fn new(wire: Wire) -> Self {
        Self { wire }
    }
}

impl ClientSource for Source {
    type Client = Wire;

    fn client(&self) -> Result<Wire, ClientError> {
        Ok(self.wire.clone())
    }
}

// One mock request end to end: the recording config, the wire that answers it once and the call's inputs
pub(crate) struct MockScenario {
    pub(crate) events: Events,
    pub(crate) wire: Wire,
    pub(crate) config: RecordingConfig,
    source: Source,
    pub(crate) params: ContentParams,
    pub(crate) headers: Headers,
    pub(crate) request: FileContentRequest,
    pub(crate) timeout: Option<Duration>,
}

impl MockScenario {
    pub(crate) fn new(answer: Answer, base: &str, query: QueryParams) -> Self {
        let events = Events::new();
        let wire = Wire::answering(answer, &events);
        Self {
            config: recording_config(base, query, provider_headers(), &events),
            source: Source::new(wire.clone()),
            wire,
            events,
            params: params("http://mock.test/v1"),
            headers: Headers::new(),
            request: request("f", IndexMap::new()),
            timeout: None,
        }
    }

    pub(crate) async fn buffered(&self) -> (Fixture, Result<BinaryContent, ContentError>) {
        let mut fixture = Fixture::new("file_content");
        let content = fixture
            .fetch(&self.deps(), &self.call(), &self.events)
            .await;
        (fixture, content)
    }

    pub(crate) async fn streaming(
        &self,
        chunk: NonZeroUsize,
    ) -> (Fixture, Result<FileContentStreamingResult, ContentError>) {
        let mut fixture = Fixture::new("file_content");
        let deps = self.deps();
        let prepared = fixture
            .prepare(&deps, &self.call(), &self.events)
            .expect("the request prepares");
        let result = retrieve_content_streaming(prepared, chunk, &mut fixture.state).await;
        (fixture, result)
    }

    const fn deps(&self) -> ContentDeps<'_, RecordingConfig, Source> {
        ContentDeps {
            config: &self.config,
            source: &self.source,
        }
    }

    fn call(&self) -> RetrieveCall<'_> {
        retrieve_call(&self.request, &self.params, &self.headers, self.timeout)
    }
}

pub(crate) struct ProxiedSource {
    client: ProxiedClient,
}

impl ProxiedSource {
    pub(crate) fn new(limits: TransportLimits) -> Self {
        Self {
            client: proxied_client(limits),
        }
    }
}

impl ClientSource for ProxiedSource {
    type Client = ProxiedClient;

    fn client(&self) -> Result<ProxiedClient, ClientError> {
        Ok(self.client.clone())
    }
}

fn proxied_client(limits: TransportLimits) -> ProxiedClient {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("the proxy policy resolves"),
    );
    ProxiedClient::build(base_builder().http1_only(), &policy, limits)
        .expect("the proxied client builds")
}

pub(crate) const IMPATIENT: TransportLimits = TransportLimits {
    read_timeout: Duration::from_millis(50),
    ..TransportLimits::DEFAULT
};

pub(crate) struct TransformRecord {
    pub(crate) file_id: String,
    pub(crate) extra_body: IndexMap<String, String>,
    pub(crate) api_base: Option<String>,
}

pub(crate) struct StreamRecord {
    pub(crate) request_url: String,
    pub(crate) headers: HttpHeaders,
    pub(crate) chunk_size: NonZeroUsize,
    pub(crate) logged_file_id: Option<String>,
}

#[derive(Error, Debug)]
#[error("the provider is unavailable")]
struct Unavailable;

pub(crate) struct RecordingConfig {
    base: String,
    query: QueryParams,
    provider: Headers,
    unavailable: bool,
    events: Events,
    pub(crate) requests: Mutex<Vec<TransformRecord>>,
    pub(crate) caller_headers: Mutex<Vec<Headers>>,
    pub(crate) api_keys: Mutex<Vec<Option<String>>>,
    pub(crate) responses: Mutex<Vec<BufferedResponse>>,
    pub(crate) streams: Mutex<Vec<StreamRecord>>,
    pub(crate) errors: Mutex<Vec<(String, u16, Headers)>>,
}

pub(crate) fn recording_config(
    base: &str,
    query: QueryParams,
    provider: Headers,
    events: &Events,
) -> RecordingConfig {
    RecordingConfig {
        base: base.to_owned(),
        query,
        provider,
        unavailable: false,
        events: events.clone(),
        requests: Mutex::default(),
        caller_headers: Mutex::default(),
        api_keys: Mutex::default(),
        responses: Mutex::default(),
        streams: Mutex::default(),
        errors: Mutex::default(),
    }
}

pub(crate) fn unavailable_config(events: &Events) -> RecordingConfig {
    let mut config = recording_config(
        "http://unused.test/files",
        QueryParams::new(),
        provider_headers(),
        events,
    );
    config.unavailable = true;
    config
}

pub(crate) fn provider_headers() -> Headers {
    Headers::from([
        (String::from("api_key"), String::from("sk-provider")),
        (
            String::from("content-type"),
            String::from("application/json"),
        ),
    ])
}

impl RecordingConfig {
    pub(crate) fn errors(&self) -> Vec<(String, u16, Headers)> {
        self.errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl FileContentConfig for RecordingConfig {
    fn transform_content_request(
        &self,
        request: &FileContentRequest,
        params: &ContentParams,
    ) -> Result<(String, QueryParams), ContentError> {
        if self.unavailable {
            return Err(ContentError::Transform(Box::new(Unavailable)));
        }
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(TransformRecord {
                file_id: request.file_id.clone(),
                extra_body: request.extra_body.clone(),
                api_base: params.api_base.clone(),
            });
        Ok((self.base.clone(), self.query.clone()))
    }

    fn validate_environment(
        &self,
        headers: Headers,
        params: &ContentParams,
    ) -> Result<Headers, ContentError> {
        self.caller_headers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(headers.clone());
        self.api_keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(
                params
                    .api_key
                    .as_ref()
                    .map(|key| key.expose_secret().to_owned()),
            );
        let mut provided = headers;
        for (name, value) in &self.provider {
            provided.insert(name.clone(), value.clone());
        }
        if let Some(key) = &params.api_key {
            provided.insert(String::from("x-params-key"), key.expose_secret().to_owned());
        }
        Ok(provided)
    }

    fn transform_content_response(
        &self,
        response: &BufferedResponse,
        _logging: &mut LoggingState,
        _params: &ContentParams,
    ) -> Result<BinaryContent, ContentError> {
        self.events.push("transform");
        self.responses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(response.clone());
        let mut headers: IndexMap<String, String> = response
            .headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        headers.insert(String::from("x-transform"), String::from("recorded"));
        Ok(BinaryContent {
            status: response.status_code,
            headers,
            body: response.body.clone(),
        })
    }

    fn transform_content_stream(
        &self,
        body: BodyStream,
        headers: &HttpHeaders,
        chunk_size: NonZeroUsize,
        context: StreamContext<'_>,
    ) -> FileContentStreamingResult {
        self.events.push("transform");
        let logged_file_id = logged_text(context.logging, "file_id");
        self.streams
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(StreamRecord {
                request_url: context.request_url.to_owned(),
                headers: headers.clone(),
                chunk_size,
                logged_file_id,
            });
        let mut marked = headers.clone();
        marked.insert("x-transform", HeaderValue::from_static("recorded"));
        FileContentStreamingResult::new(body, &marked, Some(chunk_size))
    }

    fn error_class(&self, message: &str, status_code: u16, headers: Headers) -> BaseLlmException {
        self.errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((message.to_owned(), status_code, headers.clone()));
        BaseLlmException {
            status_code,
            message: message.to_owned(),
            headers: headers
                .into_iter()
                .map(|(name, value)| (name, Json::Text(value)))
                .collect(),
            status_code_is_synthesized: false,
        }
    }
}

fn logged_text(logging: &LoggingState, name: &str) -> Option<String> {
    let Some(Json::Object(details)) = logging.model_call_details.get("additional_args") else {
        return None;
    };
    let Some(Json::Text(text)) = details.get(name) else {
        return None;
    };
    Some(text.clone())
}

pub(crate) type Chunks = Sender<Result<Vec<u8>, HttpError>>;

pub(crate) struct Counted {
    inner: BodyStream,
    drops: Arc<AtomicUsize>,
}

impl Stream for Counted {
    type Item = Result<Vec<u8>, HttpError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().inner.poll_next_unpin(cx)
    }
}

impl Drop for Counted {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

// A body the test feeds one chunk at a time, so the service can return before the first byte exists
pub(crate) fn gated(drops: &Arc<AtomicUsize>, capacity: usize) -> (Chunks, BodyStream) {
    let (sender, mut receiver): (Chunks, Receiver<Result<Vec<u8>, HttpError>>) = channel(capacity);
    let inner = poll_fn(move |cx| receiver.poll_recv(cx)).boxed();
    let counted = Counted {
        inner,
        drops: Arc::clone(drops),
    };
    let stream: BoxStream<'static, Result<Vec<u8>, HttpError>> = counted.boxed();
    (sender, stream)
}

pub(crate) fn params(api_base: &str) -> ContentParams {
    ContentParams {
        api_base: Some(api_base.to_owned()),
        api_key: None,
    }
}

pub(crate) fn caller_headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn extra_body(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn request(file_id: &str, extra_body: IndexMap<String, String>) -> FileContentRequest {
    FileContentRequest {
        file_id: file_id.to_owned(),
        extra_headers: IndexMap::new(),
        extra_body,
        timeout: None,
    }
}

pub(crate) type HeaderPairs = [(&'static str, &'static str)];

pub(crate) fn http_headers(pairs: &HeaderPairs) -> HttpHeaders {
    let mut headers = HttpHeaders::new();
    for (name, value) in pairs {
        headers.append(
            *name,
            HeaderValue::from_str(value).expect("the header value"),
        );
    }
    headers
}

pub(crate) fn buffered(status: u16, headers: &HeaderPairs, body: &[u8]) -> Answer {
    Answer::Buffered(status, http_headers(headers), body.to_vec())
}

pub(crate) fn streaming(status: u16, headers: &HeaderPairs, body: BodyStream) -> Answer {
    Answer::Stream(status, http_headers(headers), body)
}

pub(crate) fn key(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

pub(crate) fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn provider_failure(error: ContentError) -> BaseLlmException {
    let ContentError::Provider(failure) = error else {
        panic!("the request fails with a provider error");
    };
    failure
}

pub(crate) fn error_headers(pairs: &[(&str, &str)]) -> ErrorHeaders {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), Json::Text((*value).to_owned())))
        .collect()
}

pub(crate) fn retrieve_call<'call>(
    request: &'call FileContentRequest,
    params: &'call ContentParams,
    headers: &'call Headers,
    timeout: Option<Duration>,
) -> RetrieveCall<'call> {
    RetrieveCall {
        request,
        params,
        headers,
        timeout,
    }
}

pub(crate) fn wire_header<'request>(
    request: &'request ContentRequest,
    name: &str,
) -> Option<&'request str> {
    let value = request.headers.get(name)?;
    value.to_str().ok()
}
