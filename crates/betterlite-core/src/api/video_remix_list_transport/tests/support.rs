use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::base_builder;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
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
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use http::HeaderMap as HttpHeaders;
use secrecy::SecretString;

use super::recording::RecordingConfig;
use crate::api::video_records::VideoObject;
use crate::api::video_remix_list_transport::client::{
    VideoAsyncClient, VideoClient, VideoClientSource, VideoRequest, VideoSyncClient,
};
use crate::api::video_remix_list_transport::error::VideoRemixListTransportError;
use crate::api::video_remix_list_transport::records::{Headers, Params};
use crate::api::video_remix_list_transport::{
    Handled, ListCall, LogContext, RemixCall, VideoRemixListTransport,
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

pub(crate) type Logger = Box<dyn Fn(&Json) -> Result<(), PrePostError> + Sync>;

// The fixture's logging object: one redactor, one sink and a logger function that marks every pre-call
pub(crate) struct LogEnv {
    redactor: Redactor,
    sink: Sink,
    logger: Logger,
    now: DateTime<FixedOffset>,
}

impl LogEnv {
    fn new(events: &Events) -> Self {
        let recorded = events.clone();
        Self {
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds"),
            sink: Sink,
            logger: Box::new(move |_details: &Json| {
                recorded.push("log");
                Ok(())
            }),
            now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00")
                .expect("the clock parses"),
        }
    }

    fn pre_post(&self) -> PrePostEnv<'_> {
        PrePostEnv {
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
            logger_fn: Some(&*self.logger),
            now: self.now,
        }
    }
}

pub(crate) struct Fixture {
    pub(crate) state: LoggingState,
    env: LogEnv,
}

impl Fixture {
    pub(crate) fn new(call_type: &str, events: &Events) -> Self {
        let init: LoggingInit = serde_json::from_str(&format!(
            r#"{{"model":"video","call_type":"{call_type}","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}}"#
        ))
        .expect("the logging fixture is valid");
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
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
            state,
            env: LogEnv::new(events),
        }
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
}

#[derive(Clone)]
pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) headers: HttpHeaders,
    pub(crate) body: Vec<u8>,
}

impl Reply {
    pub(crate) fn json(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: HttpHeaders::new(),
            body: body.as_bytes().to_vec(),
        }
    }

    pub(crate) fn bytes(status: u16, body: Vec<u8>) -> Self {
        Self {
            status,
            headers: HttpHeaders::new(),
            body,
        }
    }

    pub(crate) fn with_header(mut self, name: &'static str, value: &str) -> Self {
        self.headers.insert(
            name,
            http::HeaderValue::from_str(value).expect("the header value"),
        );
        self
    }
}

#[derive(Clone)]
pub(crate) struct Wire {
    reply: Reply,
    failure: Arc<Mutex<Option<TransportError>>>,
    requests: Arc<Mutex<Vec<VideoRequest>>>,
    events: Events,
}

impl Wire {
    pub(crate) fn replying(reply: Reply, events: &Events) -> Self {
        Self {
            reply,
            failure: Arc::default(),
            requests: Arc::default(),
            events: events.clone(),
        }
    }

    pub(crate) fn requests(&self) -> MutexGuard<'_, Vec<VideoRequest>> {
        self.requests.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn answer(&self, request: VideoRequest) -> Result<UpstreamResponse, TransportError> {
        self.events.push("send");
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        let failure = self
            .failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(error) = failure {
            return Err(error);
        }
        Ok(UpstreamResponse {
            status: self.reply.status,
            headers: self.reply.headers.clone(),
            body: self.reply.body.clone(),
        })
    }
}

impl VideoSyncClient for Wire {
    fn send(&self, request: VideoRequest) -> Result<UpstreamResponse, TransportError> {
        self.answer(request)
    }
}

impl VideoAsyncClient for Wire {
    fn default_timeout(&self) -> Duration {
        Duration::from_secs(60)
    }

    fn send(
        &self,
        request: VideoRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        ready(self.answer(request))
    }
}

#[derive(Clone)]
pub(crate) struct Source {
    wire: Wire,
    fail: bool,
    sync_calls: Arc<Mutex<Vec<Option<bool>>>>,
    async_calls: Arc<Mutex<Vec<(ProviderId, Option<bool>)>>>,
}

impl Source {
    pub(crate) fn new(wire: Wire) -> Self {
        Self {
            wire,
            fail: false,
            sync_calls: Arc::default(),
            async_calls: Arc::default(),
        }
    }

    pub(crate) fn failing(wire: Wire) -> Self {
        Self {
            fail: true,
            ..Self::new(wire)
        }
    }

    pub(crate) fn sync_calls(&self) -> Vec<Option<bool>> {
        self.sync_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn async_calls(&self) -> Vec<(ProviderId, Option<bool>)> {
        self.async_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn record<T>(calls: &Mutex<Vec<T>>, value: T) {
    let mut held = calls.lock().unwrap_or_else(PoisonError::into_inner);
    held.push(value);
}

impl VideoClientSource for Source {
    type SyncClient = Wire;
    type AsyncClient = Wire;
    type Error = std::io::Error;

    fn sync_client(&self, ssl_verify: Option<bool>) -> Result<Wire, std::io::Error> {
        record(&self.sync_calls, ssl_verify);
        self.wire_or_error()
    }

    fn async_client(
        &self,
        provider: ProviderId,
        ssl_verify: Option<bool>,
    ) -> Result<Wire, std::io::Error> {
        record(&self.async_calls, (provider, ssl_verify));
        self.wire_or_error()
    }
}

impl Source {
    fn wire_or_error(&self) -> Result<Wire, std::io::Error> {
        if self.fail {
            return Err(std::io::Error::other("the pool is down"));
        }
        Ok(self.wire.clone())
    }
}

pub(crate) struct UnusableSync;

impl VideoSyncClient for UnusableSync {
    fn send(&self, _request: VideoRequest) -> Result<UpstreamResponse, TransportError> {
        panic!("the proxied fixture has no blocking client")
    }
}

// The real B173 transport for the mock-server cases: only its async client is used
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

impl VideoClientSource for ProxiedSource {
    type SyncClient = UnusableSync;
    type AsyncClient = ProxiedClient;
    type Error = std::io::Error;

    fn sync_client(&self, _ssl_verify: Option<bool>) -> Result<UnusableSync, std::io::Error> {
        panic!("the proxied fixture has no blocking client")
    }

    fn async_client(
        &self,
        _provider: ProviderId,
        _ssl_verify: Option<bool>,
    ) -> Result<ProxiedClient, std::io::Error> {
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
    read_timeout: Duration::from_millis(100),
    ..TransportLimits::DEFAULT
};

pub(crate) fn transport(source: Source) -> VideoRemixListTransport<Source> {
    VideoRemixListTransport { source }
}

pub(crate) fn proxied_transport(limits: TransportLimits) -> VideoRemixListTransport<ProxiedSource> {
    VideoRemixListTransport {
        source: ProxiedSource::new(limits),
    }
}

// One case's logging object, config and transport, all sharing the same event log
pub(crate) struct Bench<Source: VideoClientSource> {
    pub(crate) events: Events,
    pub(crate) fixture: Fixture,
    pub(crate) config: RecordingConfig,
    pub(crate) transport: VideoRemixListTransport<Source>,
}

fn bench_from(
    call_type: &str,
    events: Events,
    config: RecordingConfig,
    wire: Wire,
) -> Bench<Source> {
    Bench {
        fixture: Fixture::new(call_type, &events),
        transport: transport(Source::new(wire.clone())),
        events,
        config,
    }
}

pub(crate) fn fake_bench<Configure>(
    call_type: &str,
    reply: Reply,
    configure: Configure,
) -> (Bench<Source>, Wire)
where
    Configure: FnOnce(&Events) -> RecordingConfig,
{
    let events = Events::new();
    let config = configure(&events);
    let wire = Wire::replying(reply, &events);
    (bench_from(call_type, events, config, wire.clone()), wire)
}

pub(crate) fn failing(wire: &Wire, error: TransportError) {
    let mut failure = wire.failure.lock().unwrap_or_else(PoisonError::into_inner);
    *failure = Some(error);
}

pub(crate) fn proxied_bench<Configure>(
    call_type: &str,
    configure: Configure,
    limits: TransportLimits,
) -> Bench<ProxiedSource>
where
    Configure: FnOnce(&Events) -> RecordingConfig,
{
    let events = Events::new();
    let config = configure(&events);
    let fixture = Fixture::new(call_type, &events);
    Bench {
        events,
        fixture,
        config,
        transport: proxied_transport(limits),
    }
}

// What validate_environment and get_complete_url were given and what they were handed back
pub(crate) fn assert_validated(
    bench: &Bench<Source>,
    api_key: Option<&str>,
    api_base: Option<&str>,
    given: &Headers,
    params: &Params,
) {
    assert_eq!(bench.config.api_keys(), vec![api_key.map(str::to_owned)]);
    assert_eq!(bench.config.models(), vec![String::new()]);
    assert_eq!(
        bench.config.urls(),
        vec![(String::new(), api_base.map(str::to_owned))]
    );
    assert_eq!(bench.config.validated_params(), vec![Some(params.clone())]);
    assert_eq!(bench.config.url_params(), vec![params.clone()]);
    assert_eq!(bench.config.validated(), vec![given.clone()]);
}

// A call with every optional argument at its handler default; a case fills in what it exercises
pub(crate) struct Setup<'call, Source: VideoClientSource> {
    pub(crate) video_id: &'call str,
    pub(crate) prompt: &'call str,
    pub(crate) provider: &'call str,
    pub(crate) params: &'call Params,
    pub(crate) client: Option<VideoClient<'call, Source>>,
    pub(crate) extra_headers: Option<&'call Headers>,
    pub(crate) extra_body: Option<&'call Params>,
    pub(crate) after: Option<&'call str>,
    pub(crate) limit: Option<i64>,
    pub(crate) order: Option<&'call str>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) extra_query: Option<&'call Params>,
    pub(crate) api_key: Option<&'call SecretString>,
}

impl<'call, Source: VideoClientSource> Setup<'call, Source> {
    pub(crate) fn new(provider: &'call str, params: &'call Params) -> Self {
        Self {
            video_id: "",
            prompt: "",
            provider,
            params,
            client: None,
            extra_headers: None,
            extra_body: None,
            after: None,
            limit: None,
            order: None,
            timeout: None,
            extra_query: None,
            api_key: None,
        }
    }

    // The remix handlers take the video and the prompt; a list case leaves both empty
    pub(crate) fn video(mut self, video_id: &'call str, prompt: &'call str) -> Self {
        self.video_id = video_id;
        self.prompt = prompt;
        self
    }

    fn into_call(self, kind: CallKind) -> FixtureCall<'call, Source> {
        match kind {
            CallKind::Remix => FixtureCall::Remix(RemixCall {
                video_id: self.video_id,
                prompt: self.prompt,
                custom_llm_provider: self.provider,
                litellm_params: self.params,
                client: self.client,
                extra_headers: self.extra_headers,
                extra_body: self.extra_body,
                timeout: self.timeout,
                api_key: self.api_key,
            }),
            CallKind::List => FixtureCall::List(ListCall {
                after: self.after,
                limit: self.limit,
                order: self.order,
                custom_llm_provider: self.provider,
                litellm_params: self.params,
                client: self.client,
                extra_headers: self.extra_headers,
                extra_query: self.extra_query,
                timeout: self.timeout,
                api_key: self.api_key,
            }),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum CallKind {
    Remix,
    List,
}

enum FixtureCall<'call, Source: VideoClientSource> {
    Remix(RemixCall<'call, Source>),
    List(ListCall<'call, Source>),
}

// The handler's answer; the object is boxed so the two variants cost about the same
#[derive(Debug)]
pub(crate) enum Answer {
    Object(Box<VideoObject>),
    Page(Params),
}

impl Answer {
    pub(crate) fn object(self) -> VideoObject {
        match self {
            Self::Object(object) => *object,
            Self::Page(_) => panic!("the remix handlers answer with the video object"),
        }
    }

    pub(crate) fn page(self) -> Params {
        match self {
            Self::Page(page) => page,
            Self::Object(_) => panic!("the list handlers answer with the params"),
        }
    }
}

pub(crate) const LIST_PAGE: &str = r#"{"data":[]}"#;

pub(crate) fn json_params(text: &str) -> Params {
    serde_json::from_str(text).expect("the params fixture is valid")
}

pub(crate) fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn video_object(id: &str, status: &str) -> VideoObject {
    VideoObject::new(id.to_owned(), status.to_owned())
}

pub(crate) fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

pub(crate) fn request_body(request: &VideoRequest) -> Json {
    let RequestBody::Json(body) = &request.body else {
        panic!("the video POST carries a JSON body");
    };
    serde_json::from_slice(body).expect("the request body is JSON")
}

pub(crate) fn request_header<'request>(
    request: &'request VideoRequest,
    name: &str,
) -> &'request str {
    request
        .headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .expect("the request carries the header as text")
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum CallMode {
    Sync,
    Asynchronous,
    HandOff,
}

// The handler behind the fixture's own logging object, in the mode the case asks for
pub(crate) async fn run<Source>(
    bench: &mut Bench<Source>,
    setup: Setup<'_, Source>,
    kind: CallKind,
    mode: CallMode,
) -> Result<Answer, VideoRemixListTransportError>
where
    Source: VideoClientSource,
{
    let env = bench.fixture.env.pre_post();
    let log = LogContext {
        state: &mut bench.fixture.state,
        env: &env,
    };
    let pool = &bench.transport;
    let config = &bench.config;
    match setup.into_call(kind) {
        FixtureCall::Remix(call) => match mode {
            CallMode::Asynchronous => {
                let object = pool.async_remix(config, call, log).await?;
                Ok(Answer::Object(Box::new(object)))
            }
            CallMode::Sync => {
                let object = done(pool.remix(config, call, log, false)?);
                Ok(Answer::Object(Box::new(object)))
            }
            CallMode::HandOff => {
                let object = pending(pool.remix(config, call, log, true)?).await?;
                Ok(Answer::Object(Box::new(object)))
            }
        },
        FixtureCall::List(call) => match mode {
            CallMode::Asynchronous => Ok(Answer::Page(pool.async_list(config, call, log).await?)),
            CallMode::Sync => Ok(Answer::Page(done(pool.list(config, call, log, false)?))),
            CallMode::HandOff => {
                let page = pending(pool.list(config, call, log, true)?).await?;
                Ok(Answer::Page(page))
            }
        },
    }
}

fn pending<Done, Pending>(handled: Handled<Done, Pending>) -> Pending {
    match handled {
        Handled::Pending(pending) => pending,
        Handled::Done(_) => panic!("_is_async hands back the async handler's future"),
    }
}

fn done<Done, Pending>(handled: Handled<Done, Pending>) -> Done {
    match handled {
        Handled::Done(finished) => finished,
        Handled::Pending(_) => panic!("the sync handler answers with its own result"),
    }
}

// The sync list handler builds its own runtime, so its happy case calls it before any runtime exists
pub(crate) fn run_sync_list<Source>(
    bench: &mut Bench<Source>,
    setup: Setup<'_, Source>,
) -> Result<Params, VideoRemixListTransportError>
where
    Source: VideoClientSource,
{
    let env = bench.fixture.env.pre_post();
    let log = LogContext {
        state: &mut bench.fixture.state,
        env: &env,
    };
    let FixtureCall::List(call) = setup.into_call(CallKind::List) else {
        panic!("the sync list helper is given a list call");
    };
    Ok(done(bench.transport.list(
        &bench.config,
        call,
        log,
        false,
    )?))
}
