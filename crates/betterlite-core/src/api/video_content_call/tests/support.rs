use std::future::{Future, pending, ready as future_ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_error::error::ErrorClass;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use thiserror::Error;

use crate::api::video_content_call::error::VideoContentCallError;
use crate::api::video_content_call::handler::{
    ContentHandlerEnvironment, ContentHandlerRequest, ContentTransportHandler, VideoContentHandler,
};
use crate::api::video_content_call::providers::{Selection, VideoContentProviders};
use crate::api::video_content_call::{
    PendingContent, VideoContentCaller, VideoContentRequest, VideoContentWork,
};
use crate::api::video_records::VideoObject;
use crate::api::video_selection::error::VideoSelectionError;
use crate::api::videos_http_transport::client::{
    ClientError, ClientKind, ClientRequest, ClientSource, SuppliedClient, VideoRequest,
    VideoTransport,
};
use crate::api::videos_http_transport::video_config::{
    ErrorFacts, Headers, Params, RequestFiles, VideoEnvironment, VideoHttpConfig, VideoLogging,
    VideoReply, VideoRequestContext,
};

pub(crate) const VIDEO_ID: &str = "video-1";
pub(crate) const PROVIDER: &str = "gemini";
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_mins(100);
pub(crate) const WIRE_BODY: &str = "video-bytes";

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
    Handled(Box<Handled>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Handled {
    pub(crate) video_id: String,
    pub(crate) custom_llm_provider: String,
    pub(crate) litellm_params: IndexMap<String, Json>,
    pub(crate) extra_headers: Option<Headers>,
    pub(crate) variant: Option<String>,
    pub(crate) timeout: Duration,
    pub(crate) client: Option<ClientKind>,
    pub(crate) is_async: bool,
    pub(crate) logged: Logged,
}

// the logging state as the handler finds it, so the update's position and payload both show
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

pub(crate) struct FakeProviders {
    pub(crate) journal: Journal,
    pub(crate) behavior: Behavior,
    pub(crate) config: RecordingConfig,
    pub(crate) source: WireSource,
}

impl FakeProviders {
    pub(crate) fn new(journal: &Journal, behavior: Behavior) -> Self {
        Self::with_config(journal, behavior, RecordingConfig::new())
    }

    pub(crate) fn with_config(
        journal: &Journal,
        behavior: Behavior,
        config: RecordingConfig,
    ) -> Self {
        Self {
            journal: journal.clone(),
            behavior,
            config,
            source: WireSource::new(),
        }
    }
}

impl VideoContentProviders for FakeProviders {
    type Config = RecordingConfig;
    type Source = WireSource;

    fn video_config(
        &self,
        provider: ProviderId,
    ) -> Result<Selection<'_, Self>, VideoSelectionError> {
        self.journal.push(Event::Lookup {
            provider: provider.to_string(),
        });
        match self.behavior {
            Behavior::Found => Ok(Some((&self.config, &self.source))),
            Behavior::Missing => Ok(None),
            Behavior::Unavailable => Err(VideoSelectionError::Unavailable {
                provider: provider.to_string(),
            }),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Outcome {
    Bytes(Vec<u8>),
    Fails { class: ErrorClass, message: String },
    NeverAnswers,
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

impl<Config, Source> VideoContentHandler<Config, Source> for FakeHandler
where
    Config: VideoHttpConfig,
    Source: ClientSource,
{
    fn handle<'call>(
        &'call self,
        environment: ContentHandlerEnvironment<'call, Config, Source>,
        request: ContentHandlerRequest<'call, Source::Client>,
        is_async: bool,
    ) -> PendingContent<'call> {
        self.journal.push(Event::Handled(Box::new(Handled {
            video_id: request.video_id.clone(),
            custom_llm_provider: request.custom_llm_provider.clone(),
            litellm_params: request.litellm_params.clone(),
            extra_headers: request.extra_headers.clone(),
            variant: request.variant.clone(),
            timeout: request.timeout,
            client: request.client.as_ref().map(|supplied| supplied.kind),
            is_async,
            logged: logged(environment.logging),
        })));
        match &self.outcome {
            Outcome::Bytes(bytes) => Box::pin(
                future_ready::<Result<Vec<u8>, VideoContentCallError>>(Ok(bytes.clone())),
            ),
            Outcome::Fails { class, message } => {
                let error = VideoContentCallError::Mapped {
                    class: class.clone(),
                    message: message.clone(),
                };
                Box::pin(future_ready::<Result<Vec<u8>, VideoContentCallError>>(Err(
                    error,
                )))
            }
            Outcome::NeverAnswers => Box::pin(pending()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub(crate) enum TestError {
    #[error("the transform refused")]
    Refused,
}

pub(crate) struct RecordingConfig {
    pub(crate) environments: Mutex<Vec<Environment>>,
    pub(crate) contexts: Mutex<Vec<(String, Option<String>, Headers)>>,
    pub(crate) replies: Mutex<Vec<(u16, Vec<u8>)>>,
    pub(crate) failure: Option<TestError>,
    pub(crate) failure_status: Option<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Environment {
    pub(crate) headers: Headers,
    pub(crate) model: String,
    pub(crate) api_key: Option<String>,
    pub(crate) litellm_params: Option<Params>,
}

impl RecordingConfig {
    pub(crate) fn environments(&self) -> Vec<Environment> {
        let guard = self
            .environments
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        guard.clone()
    }

    pub(crate) fn contexts(&self) -> Vec<(String, Option<String>, Headers)> {
        let guard = self.contexts.lock().unwrap_or_else(PoisonError::into_inner);
        guard.clone()
    }

    pub(crate) fn replies(&self) -> Vec<(u16, Vec<u8>)> {
        let guard = self.replies.lock().unwrap_or_else(PoisonError::into_inner);
        guard.clone()
    }

    pub(crate) fn new() -> Self {
        Self {
            environments: Mutex::default(),
            contexts: Mutex::default(),
            replies: Mutex::default(),
            failure: None,
            failure_status: None,
        }
    }

    pub(crate) fn failing() -> Self {
        Self {
            failure: Some(TestError::Refused),
            failure_status: Some(400),
            ..Self::new()
        }
    }
}

impl VideoHttpConfig for RecordingConfig {
    type Error = TestError;

    fn validate_environment(&self, request: VideoEnvironment<'_>) -> Headers {
        self.environments
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Environment {
                headers: request.headers.clone(),
                model: request.model.to_owned(),
                api_key: request.api_key.map(|key| key.expose_secret().to_owned()),
                litellm_params: request.litellm_params.cloned(),
            });
        let mut headers = Headers::new();
        headers.insert("x-provider".to_owned(), "gemini".to_owned());
        headers
    }

    fn get_complete_url(
        &self,
        _model: &str,
        api_base: Option<&str>,
        _litellm_params: &Params,
    ) -> Result<String, TestError> {
        Ok(api_base.unwrap_or("http://mock.test/videos").to_owned())
    }

    fn transform_video_create_request(
        &self,
        _model: &str,
        _prompt: &str,
        _optional_params: &Params,
        _context: VideoRequestContext<'_>,
    ) -> Result<(Params, RequestFiles, String), TestError> {
        Err(TestError::Refused)
    }

    fn transform_video_create_response(
        &self,
        _model: &str,
        _reply: VideoReply<'_>,
        _request_data: Option<&Params>,
    ) -> Result<VideoObject, TestError> {
        Err(TestError::Refused)
    }

    fn transform_video_content_request(
        &self,
        video_id: &str,
        variant: Option<&str>,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), TestError> {
        self.contexts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((
                context.api_base.to_owned(),
                variant.map(str::to_owned),
                context.headers.clone(),
            ));
        Ok((format!("{}/{video_id}", context.api_base), Params::new()))
    }

    fn transform_video_content_response(
        &self,
        raw_response: &UpstreamResponse,
        _logging_obj: &mut VideoLogging,
    ) -> Result<Vec<u8>, TestError> {
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((raw_response.status, raw_response.body.clone()));
        if self.failure == Some(TestError::Refused) {
            return Err(TestError::Refused);
        }
        Ok(raw_response.body.clone())
    }

    fn error_facts(&self, error: &TestError) -> ErrorFacts {
        ErrorFacts {
            message: error.to_string(),
            status_code: self.failure_status,
            headers: Headers::new(),
        }
    }

    fn get_error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: &Headers,
    ) -> ProviderError {
        ProviderError::from(ProviderErrorParts {
            status_code,
            message: error_message.to_owned(),
            headers: Some(
                headers
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .collect(),
            ),
            request: None,
            response: None,
            body: None,
            status_code_is_synthesized: false,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Seen {
    pub(crate) method: String,
    pub(crate) url: String,
    pub(crate) headers: Vec<(String, String)>,
}

pub(crate) struct Wire {
    pub(crate) seen: Mutex<Vec<Seen>>,
}

impl Wire {
    pub(crate) fn new() -> Self {
        Self {
            seen: Mutex::default(),
        }
    }

    pub(crate) fn seen(&self) -> Vec<Seen> {
        let guard = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        guard.clone()
    }
}

impl VideoTransport for Wire {
    fn send(
        &self,
        request: VideoRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let headers: Vec<(String, String)> = request
            .headers
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|shown| (name.as_str().to_owned(), shown.to_owned()))
            })
            .collect();
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Seen {
                method: request.method.to_string(),
                url: request.url.to_string(),
                headers,
            });
        future_ready::<Result<UpstreamResponse, TransportError>>(Ok(UpstreamResponse {
            status: 200,
            headers: http::HeaderMap::new(),
            body: WIRE_BODY.as_bytes().to_vec(),
        }))
    }
}

pub(crate) struct WireSource {
    pub(crate) calls: Mutex<Vec<ClientKind>>,
}

impl WireSource {
    pub(crate) fn new() -> Self {
        Self {
            calls: Mutex::default(),
        }
    }

    pub(crate) fn calls(&self) -> Vec<ClientKind> {
        let guard = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        guard.clone()
    }
}

impl ClientSource for WireSource {
    type Client = Wire;

    fn client(&self, request: ClientRequest<'_>) -> Result<Wire, ClientError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.kind);
        Ok(Wire::new())
    }
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

pub(crate) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds")
}

pub(crate) struct Bench {
    pub(crate) caller: VideoContentCaller<FakeProviders, FakeHandler>,
    pub(crate) fixture: Fixture,
    pub(crate) journal: Journal,
}

impl Bench {
    pub(crate) fn new(outcome: Outcome) -> Self {
        Self::with(Behavior::Found, outcome)
    }

    pub(crate) fn with(behavior: Behavior, outcome: Outcome) -> Self {
        let journal = Journal::new();
        Self {
            caller: VideoContentCaller {
                providers: FakeProviders::new(&journal, behavior),
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
        request: VideoContentRequest,
        client: Option<SuppliedClient<'bench, Wire>>,
    ) -> ContentWork<'bench> {
        self.caller
            .video_content(request, client, &mut self.fixture.state)
    }
}

pub(crate) fn request(video_id: &str) -> VideoContentRequest {
    VideoContentRequest {
        video_id: video_id.to_owned(),
        timeout: None,
        custom_llm_provider: Some(PROVIDER.to_owned()),
        variant: None,
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

pub(crate) type ContentWork<'bench> = Result<VideoContentWork<'bench>, VideoContentCallError>;

pub(crate) fn call(bench: &mut Bench, request: VideoContentRequest) -> ContentWork<'_> {
    bench.run(request, None)
}

pub(crate) fn call_with<'bench>(
    bench: &'bench mut Bench,
    request: VideoContentRequest,
    client: SuppliedClient<'bench, Wire>,
) -> ContentWork<'bench> {
    bench.run(request, Some(client))
}

// the caller wired to B3333's own handler, for the cases that reach a wire
pub(crate) fn transport_caller(
    journal: &Journal,
    config: RecordingConfig,
) -> VideoContentCaller<FakeProviders, ContentTransportHandler> {
    VideoContentCaller {
        providers: FakeProviders::with_config(journal, Behavior::Found, config),
        handler: ContentTransportHandler,
        redactor: redactor(),
        request_timeout: REQUEST_TIMEOUT,
    }
}

pub(crate) async fn async_call(
    bench: &mut Bench,
    request: VideoContentRequest,
) -> Result<Vec<u8>, VideoContentCallError> {
    bench
        .caller
        .avideo_content(request, None, &mut bench.fixture.state)
        .await
}

pub(crate) fn ready(work: Result<VideoContentWork<'_>, VideoContentCallError>) -> Vec<u8> {
    let Ok(VideoContentWork::Ready(bytes)) = work else {
        panic!("the call answers with the content");
    };
    bytes
}

pub(crate) fn failed(
    work: Result<VideoContentWork<'_>, VideoContentCallError>,
) -> VideoContentCallError {
    let Err(error) = work else {
        panic!("the call fails");
    };
    error
}

pub(crate) fn pending_work(
    work: Result<VideoContentWork<'_>, VideoContentCallError>,
) -> PendingContent<'_> {
    let Ok(VideoContentWork::Pending(pending)) = work else {
        panic!("async_call asks for the pending handler");
    };
    pending
}

pub(crate) async fn pending_bytes(
    work: Result<VideoContentWork<'_>, VideoContentCallError>,
) -> Vec<u8> {
    pending_work(work)
        .await
        .expect("the pending handler answers")
}
