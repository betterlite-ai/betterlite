use std::future::{pending, ready as future_ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_error::error::ErrorClass;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::pool::PoolError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use http::HeaderMap;
use indexmap::IndexMap;

use crate::api::video_records::VideoObject;
use crate::api::video_selection::error::VideoSelectionError;
use crate::api::video_status_call::error::VideoStatusCallError;
use crate::api::video_status_call::handler::{
    StatusHandlerEnvironment, StatusHandlerRequest, VideoStatusHandling,
};
use crate::api::video_status_call::providers::VideoStatusProviders;
use crate::api::video_status_call::{
    PendingVideoStatus, VideoStatusCaller, VideoStatusRequest, VideoStatusWork,
};
use crate::api::video_status_transport::clients::{AsyncClient, ClientFactories, SyncClient};
use crate::api::video_status_transport::config::{
    StatusConfig, VideoErrorClasses, VideoStatusConfig,
};
use crate::api::video_status_transport::error::{
    BaseLlmException, ErrorConverter, ProviderFailure, StatusFailure,
};
use crate::api::video_status_transport::handler::VideoStatusHandler;
use crate::api::video_status_transport::records::{
    Headers, Params, StatusRequest, SuppliedClient, VideoEnvironment, VideoReply,
    VideoRequestContext,
};

pub(crate) const VIDEO_ID: &str = "video-1";
pub(crate) const PROVIDER: &str = "gemini";
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_mins(100);
pub(crate) const MOCK_JSON: &str = r#"{"id":"mock-1","object":"video","status":"completed"}"#;

#[derive(Clone, Default)]
pub(crate) struct Journal(Arc<Mutex<Vec<Event>>>);

impl Journal {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn push(&self, event: Event) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }

    pub(crate) fn events(&self) -> Vec<Event> {
        let guard = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        guard.clone()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Event {
    Lookup { provider: String },
    Validate { headers: Headers, api_key: bool },
    Transform { video_id: String, headers: Headers },
    Handled(Box<Handled>),
    SyncClient,
    AsyncClient,
    Sent(StatusRequest),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Handled {
    pub(crate) video_id: String,
    pub(crate) custom_llm_provider: String,
    pub(crate) litellm_params: Params,
    pub(crate) extra_headers: Option<Headers>,
    pub(crate) extra_body: Option<Params>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) client: Option<&'static str>,
    pub(crate) is_async: bool,
    pub(crate) logged: Logged,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Logged {
    pub(crate) model: String,
    pub(crate) user: Option<String>,
    pub(crate) call_type: String,
    pub(crate) optional_params: IndexMap<String, Json>,
    pub(crate) litellm_params: IndexMap<String, Json>,
    pub(crate) custom_llm_provider: Option<String>,
    pub(crate) provider_note: Option<Json>,
}

impl Event {
    fn into_handled(self) -> Option<Handled> {
        if let Self::Handled(handled) = self {
            Some(*handled)
        } else {
            None
        }
    }

    fn into_lookup(self) -> Option<String> {
        if let Self::Lookup { provider } = self {
            Some(provider)
        } else {
            None
        }
    }

    fn into_validation(self) -> Option<(Headers, bool)> {
        if let Self::Validate { headers, api_key } = self {
            Some((headers, api_key))
        } else {
            None
        }
    }

    fn into_transform(self) -> Option<(String, Headers)> {
        if let Self::Transform { video_id, headers } = self {
            Some((video_id, headers))
        } else {
            None
        }
    }

    fn into_sent(self) -> Option<StatusRequest> {
        if let Self::Sent(request) = self {
            Some(request)
        } else {
            None
        }
    }
}

pub(crate) fn handles(journal: &Journal) -> Vec<Handled> {
    journal
        .events()
        .into_iter()
        .filter_map(Event::into_handled)
        .collect()
}

pub(crate) fn handled(journal: &Journal) -> Handled {
    let mut handlings = handles(journal);
    assert_eq!(handlings.len(), 1, "the handler ran once");
    handlings.remove(0)
}

pub(crate) fn lookups(journal: &Journal) -> Vec<String> {
    journal
        .events()
        .into_iter()
        .filter_map(Event::into_lookup)
        .collect()
}

pub(crate) fn validations(journal: &Journal) -> Vec<(Headers, bool)> {
    journal
        .events()
        .into_iter()
        .filter_map(Event::into_validation)
        .collect()
}

pub(crate) fn transforms(journal: &Journal) -> Vec<(String, Headers)> {
    journal
        .events()
        .into_iter()
        .filter_map(Event::into_transform)
        .collect()
}

pub(crate) fn sent(journal: &Journal) -> Vec<StatusRequest> {
    journal
        .events()
        .into_iter()
        .filter_map(Event::into_sent)
        .collect()
}

fn logged(state: &LoggingState) -> Logged {
    Logged {
        model: state.model.clone(),
        user: state.user.clone(),
        call_type: state.call_type.clone(),
        optional_params: state.optional_params.clone(),
        litellm_params: state.litellm_params.clone(),
        custom_llm_provider: state.custom_llm_provider.clone(),
        provider_note: state.model_call_details.get("custom_llm_provider").cloned(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Behavior {
    Found,
    Missing,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Answers,
    Fails,
}

pub(crate) struct FakeProviders {
    pub(crate) journal: Journal,
    pub(crate) behavior: Behavior,
    pub(crate) config: FakeConfig,
}

impl FakeProviders {
    pub(crate) fn new(journal: &Journal, behavior: Behavior, mode: Mode) -> Self {
        Self {
            journal: journal.clone(),
            behavior,
            config: FakeConfig::new(journal, mode),
        }
    }
}

impl VideoStatusProviders for FakeProviders {
    type Factories = FakeFactories;

    fn video_config(
        &self,
        provider: ProviderId,
    ) -> Result<Option<&StatusConfig<'_, FakeFactories>>, VideoSelectionError> {
        self.journal.push(Event::Lookup {
            provider: provider.to_string(),
        });
        match self.behavior {
            Behavior::Found => {
                let config: &StatusConfig<'_, FakeFactories> = &self.config;
                Ok(Some(config))
            }
            Behavior::Missing => Ok(None),
            Behavior::Unavailable => Err(VideoSelectionError::Unavailable {
                provider: provider.to_string(),
            }),
        }
    }
}

pub(crate) struct FakeConfig {
    pub(crate) mode: Mode,
    pub(crate) journal: Journal,
}

impl FakeConfig {
    fn new(journal: &Journal, mode: Mode) -> Self {
        Self {
            mode,
            journal: journal.clone(),
        }
    }
}

impl VideoErrorClasses for FakeConfig {
    fn get_error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: &Headers,
    ) -> BaseLlmException {
        BaseLlmException {
            status_code,
            message: error_message.to_owned(),
            headers: headers.clone(),
            status_code_is_synthesized: false,
        }
    }
}

#[async_trait]
impl VideoStatusConfig for FakeConfig {
    type SyncClient = SyncStub;
    type AsyncClient = AsyncStub;

    fn validate_environment(
        &self,
        request: VideoEnvironment<'_>,
    ) -> Result<Headers, ProviderFailure> {
        self.journal.push(Event::Validate {
            headers: request.headers.clone(),
            api_key: request.api_key.is_some(),
        });
        let mut headers = Headers::new();
        headers.insert("x-provider".to_owned(), "gemini".to_owned());
        Ok(headers)
    }

    fn get_complete_url(
        &self,
        _model: &str,
        api_base: Option<&str>,
        _litellm_params: &Params,
    ) -> Result<String, ProviderFailure> {
        Ok(api_base.unwrap_or("http://mock.test/videos").to_owned())
    }

    fn transform_video_status_retrieve_request(
        &self,
        video_id: &str,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), ProviderFailure> {
        self.journal.push(Event::Transform {
            video_id: video_id.to_owned(),
            headers: context.headers.clone(),
        });
        Ok((format!("{}/{video_id}", context.api_base), Params::new()))
    }

    fn transform_video_status_retrieve_response(
        &self,
        _reply: VideoReply<'_>,
        _client: Option<&SyncStub>,
    ) -> Result<VideoObject, ProviderFailure> {
        answered(self.mode)
    }

    async fn async_transform_video_status_retrieve_response(
        &self,
        _reply: VideoReply<'_>,
        _client: Option<&AsyncStub>,
    ) -> Result<VideoObject, ProviderFailure> {
        answered(self.mode)
    }
}

fn answered(mode: Mode) -> Result<VideoObject, ProviderFailure> {
    match mode {
        Mode::Answers => Ok(video(VIDEO_ID)),
        Mode::Fails => Err(refusal()),
    }
}

fn refusal() -> ProviderFailure {
    ProviderFailure {
        message: "the provider refused".to_owned(),
        status_code: Some(400),
        headers: None,
    }
}

pub(crate) struct SyncStub {
    pub(crate) journal: Journal,
}

impl SyncClient for SyncStub {
    fn send(&self, request: &StatusRequest) -> Result<UpstreamResponse, TransportError> {
        self.journal.push(Event::Sent(request.clone()));
        Ok(answer())
    }
}

pub(crate) struct AsyncStub {
    pub(crate) journal: Journal,
}

#[async_trait]
impl AsyncClient for AsyncStub {
    async fn send(&self, request: &StatusRequest) -> Result<UpstreamResponse, TransportError> {
        self.journal.push(Event::Sent(request.clone()));
        Ok(answer())
    }
}

fn answer() -> UpstreamResponse {
    UpstreamResponse {
        status: 200,
        headers: HeaderMap::new(),
        body: b"{}".to_vec(),
    }
}

pub(crate) struct FakeFactories {
    pub(crate) journal: Journal,
}

impl ClientFactories for FakeFactories {
    type Synchronous = SyncStub;
    type Asynchronous = AsyncStub;

    fn sync_client(&self, _ssl_verify: Option<bool>) -> Result<SyncStub, PoolError> {
        self.journal.push(Event::SyncClient);
        Ok(SyncStub {
            journal: self.journal.clone(),
        })
    }

    fn async_client(
        &self,
        _provider: ProviderId,
        _ssl_verify: Option<bool>,
    ) -> Result<AsyncStub, PoolError> {
        self.journal.push(Event::AsyncClient);
        Ok(AsyncStub {
            journal: self.journal.clone(),
        })
    }
}

pub(crate) struct Converter;

impl ErrorConverter for Converter {
    fn convert(
        &self,
        config: &dyn VideoErrorClasses,
        failure: StatusFailure<'_>,
    ) -> BaseLlmException {
        match failure {
            StatusFailure::Transform(provider) => {
                let headers = provider.headers.clone().unwrap_or_else(Headers::new);
                config.get_error_class(
                    &provider.message,
                    provider.status_code.unwrap_or(500),
                    &headers,
                )
            }
            StatusFailure::Send(error) => {
                config.get_error_class(&error.to_string(), 500, &Headers::new())
            }
        }
    }
}

pub(crate) enum Outcome {
    Video(Box<VideoObject>),
    Fails { class: ErrorClass, message: String },
}

impl Outcome {
    pub(crate) fn answers() -> Self {
        Self::Video(Box::new(video(VIDEO_ID)))
    }

    pub(crate) fn fails(class: ErrorClass, message: &str) -> Self {
        Self::Fails {
            class,
            message: message.to_owned(),
        }
    }
}

pub(crate) struct FakeHandler {
    pub(crate) journal: Journal,
    pub(crate) outcome: Outcome,
}

impl FakeHandler {
    pub(crate) fn new(journal: &Journal, outcome: Outcome) -> Self {
        Self {
            journal: journal.clone(),
            outcome,
        }
    }
}

impl<Factories: ClientFactories> VideoStatusHandling<Factories> for FakeHandler {
    fn handle<'call>(
        &'call self,
        environment: StatusHandlerEnvironment<'call, Factories>,
        request: StatusHandlerRequest<'call, Factories::Synchronous, Factories::Asynchronous>,
        is_async: bool,
    ) -> PendingVideoStatus<'call> {
        self.journal.push(Event::Handled(Box::new(Handled {
            video_id: request.video_id.clone(),
            custom_llm_provider: request.custom_llm_provider.clone(),
            litellm_params: request.litellm_params.clone(),
            extra_headers: request.extra_headers.clone(),
            extra_body: request.extra_body.clone(),
            timeout: request.timeout,
            client: request.client.as_ref().map(|supplied| match supplied {
                SuppliedClient::Sync(_) => "sync",
                SuppliedClient::Async(_) => "async",
            }),
            is_async,
            logged: logged(environment.logging),
        })));
        match &self.outcome {
            Outcome::Video(video) => Box::pin(future_ready::<
                Result<VideoObject, VideoStatusCallError>,
            >(Ok((**video).clone()))),
            Outcome::Fails { class, message } => {
                Box::pin(future_ready::<Result<VideoObject, VideoStatusCallError>>(
                    Err(VideoStatusCallError::Mapped {
                        class: class.clone(),
                        message: message.clone(),
                    }),
                ))
            }
        }
    }
}

pub(crate) struct NeverAnswers;

impl<Factories: ClientFactories> VideoStatusHandling<Factories> for NeverAnswers {
    fn handle<'call>(
        &'call self,
        _environment: StatusHandlerEnvironment<'call, Factories>,
        _request: StatusHandlerRequest<'call, Factories::Synchronous, Factories::Asynchronous>,
        _is_async: bool,
    ) -> PendingVideoStatus<'call> {
        Box::pin(pending())
    }
}

pub(crate) struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

pub(crate) struct Environment {
    redactor: Redactor,
    sink: Sink,
    now: DateTime<FixedOffset>,
}

impl Environment {
    pub(crate) fn new() -> Self {
        Self {
            redactor: redactor(),
            sink: Sink,
            now: moment(),
        }
    }

    pub(crate) fn prepost(&self) -> PrePostEnv<'_> {
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
            logger_fn: None,
            now: self.now,
        }
    }
}

pub(crate) fn moment() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").expect("the moment parses")
}

pub(crate) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds")
}

pub(crate) struct Fixture {
    pub(crate) state: LoggingState,
}

impl Fixture {
    pub(crate) fn new() -> Self {
        let init: LoggingInit = serde_json::from_str(
            r#"{"model":"video-model","call_type":"video_generation","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
        )
        .expect("the logging fixture is valid");
        let redactor = redactor();
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
        Self { state }
    }
}

pub(crate) fn video(id: &str) -> VideoObject {
    VideoObject::new(id.to_owned(), "completed".to_owned())
}

pub(crate) fn object_json(id: &str) -> Json {
    let mut fields = IndexMap::new();
    fields.insert("id".to_owned(), Json::Text(id.to_owned()));
    fields.insert("object".to_owned(), Json::Text("video".to_owned()));
    fields.insert("status".to_owned(), Json::Text("completed".to_owned()));
    Json::Object(fields)
}

pub(crate) fn request(video_id: &str) -> VideoStatusRequest {
    VideoStatusRequest {
        video_id: video_id.to_owned(),
        timeout: None,
        custom_llm_provider: Some(PROVIDER.to_owned()),
        extra_headers: None,
        extra_query: None,
        extra_body: None,
        kwargs: IndexMap::new(),
    }
}

pub(crate) fn kwargs(pairs: &[(&str, Json)]) -> IndexMap<String, Json> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

pub(crate) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(crate) fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) struct Bench {
    pub(crate) caller: VideoStatusCaller<FakeProviders, FakeHandler>,
    pub(crate) fixture: Fixture,
    pub(crate) journal: Journal,
}

impl Bench {
    pub(crate) fn new(outcome: Outcome) -> Self {
        Self::with(Behavior::Found, Mode::Answers, outcome)
    }

    pub(crate) fn with(behavior: Behavior, mode: Mode, outcome: Outcome) -> Self {
        let journal = Journal::new();
        Self {
            caller: VideoStatusCaller {
                providers: FakeProviders::new(&journal, behavior, mode),
                handler: FakeHandler::new(&journal, outcome),
                redactor: redactor(),
                request_timeout: REQUEST_TIMEOUT,
            },
            fixture: Fixture::new(),
            journal,
        }
    }

    fn run<'bench>(
        &'bench mut self,
        request: VideoStatusRequest,
        client: Option<SuppliedClient<'bench, SyncStub, AsyncStub>>,
    ) -> StatusWork<'bench> {
        self.caller
            .video_status(request, client, &mut self.fixture.state)
    }
}

pub(crate) type StatusWork<'bench> = Result<VideoStatusWork<'bench>, VideoStatusCallError>;

pub(crate) fn call(bench: &mut Bench, request: VideoStatusRequest) -> StatusWork<'_> {
    bench.run(request, None)
}

pub(crate) fn call_with<'bench>(
    bench: &'bench mut Bench,
    request: VideoStatusRequest,
    client: SuppliedClient<'bench, SyncStub, AsyncStub>,
) -> StatusWork<'bench> {
    bench.run(request, Some(client))
}

pub(crate) async fn async_call(
    bench: &mut Bench,
    request: VideoStatusRequest,
) -> Result<VideoObject, VideoStatusCallError> {
    bench
        .caller
        .avideo_status(request, None, &mut bench.fixture.state)
        .await
}

pub(crate) fn ready(work: Result<VideoStatusWork<'_>, VideoStatusCallError>) -> VideoObject {
    let Ok(VideoStatusWork::Ready(video)) = work else {
        panic!("the call answers with the video");
    };
    *video
}

pub(crate) fn failed(
    work: Result<VideoStatusWork<'_>, VideoStatusCallError>,
) -> VideoStatusCallError {
    let Err(error) = work else {
        panic!("the call fails");
    };
    error
}

pub(crate) fn pending_work(
    work: Result<VideoStatusWork<'_>, VideoStatusCallError>,
) -> PendingVideoStatus<'_> {
    let Ok(VideoStatusWork::Pending(pending)) = work else {
        panic!("async_call asks for the pending handler");
    };
    pending
}

pub(crate) async fn pending_object(
    work: Result<VideoStatusWork<'_>, VideoStatusCallError>,
) -> VideoObject {
    pending_work(work)
        .await
        .expect("the pending handler answers")
}

pub(crate) struct Wiring {
    pub(crate) factories: FakeFactories,
    pub(crate) converter: Converter,
}

impl Wiring {
    pub(crate) fn new(journal: &Journal) -> Self {
        Self {
            factories: FakeFactories {
                journal: journal.clone(),
            },
            converter: Converter,
        }
    }
}

// the production handler wiring: B3337's own handler over the fake factories, logging environment and converter
pub(crate) struct Production<'deps> {
    pub(crate) caller: VideoStatusCaller<FakeProviders, VideoStatusHandler<'deps, FakeFactories>>,
    pub(crate) fixture: Fixture,
}

impl<'deps> Production<'deps> {
    pub(crate) fn new(
        journal: &Journal,
        behavior: Behavior,
        mode: Mode,
        wiring: &'deps Wiring,
        prepost: &'deps PrePostEnv<'deps>,
    ) -> Self {
        Self {
            caller: VideoStatusCaller {
                providers: FakeProviders::new(journal, behavior, mode),
                handler: VideoStatusHandler {
                    factories: &wiring.factories,
                    logging: prepost,
                    converter: &wiring.converter,
                },
                redactor: redactor(),
                request_timeout: REQUEST_TIMEOUT,
            },
            fixture: Fixture::new(),
        }
    }
}
