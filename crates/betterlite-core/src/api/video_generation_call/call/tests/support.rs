use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, pending as never_ready, ready as future_ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use http::HeaderMap;
use indexmap::IndexMap;

use super::super::request::{Polled, poll_once};
use crate::api::video_generation_call::VideoGenerationRequest;
use crate::api::video_generation_call::call::{
    GenerationCallEnvironment, PendingVideoGeneration, VideoGenerationCaller, VideoGenerationWork,
};
use crate::api::video_generation_call::error::{VideoGenerationCallError, VideoGenerationFailure};
use crate::api::video_generation_call::handler::{
    GenerationHandlerEnvironment, GenerationHandlerRequest, VideoGenerationHandling,
};
use crate::api::video_generation_call::providers::{
    VideoGenerationConfig, VideoGenerationProviders,
};
use crate::api::video_generation_call::testing::TestingConfig;
use crate::api::video_records::VideoObject;
use crate::api::video_selection::VideoConfigTag;
use crate::api::videos_http_transport::client::{
    ClientError, ClientKind, ClientRequest, ClientSource, VideoRequest, VideoTransport,
};
use crate::api::videos_http_transport::video_config::{Headers, Params};

pub(crate) const VIDEO_ID: &str = "video-1";
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
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Event {
    Lookup { config: String },
    Handled(Box<Handled>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Handled {
    pub(crate) model: String,
    pub(crate) prompt: String,
    pub(crate) custom_llm_provider: String,
    pub(crate) litellm_params: Params,
    pub(crate) optional_params: Params,
    pub(crate) extra_headers: Option<Headers>,
    pub(crate) extra_body: Option<Params>,
    pub(crate) timeout: Duration,
    pub(crate) client: Option<ClientKind>,
    pub(crate) is_async: bool,
    pub(crate) logged: Logged,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Logged {
    pub(crate) model: String,
    pub(crate) user: Option<String>,
    pub(crate) call_type: String,
    pub(crate) optional_params: Params,
    pub(crate) litellm_params: Params,
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
        if let Self::Lookup { config } = self {
            Some(config)
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
pub(crate) enum Outcome {
    Answers,
    FieldType,
    NeverAnswers,
}

pub(crate) struct FakeProviders {
    pub(crate) config: TestingConfig,
    source: FakeSource,
    missing: bool,
    journal: Journal,
}

impl FakeProviders {
    fn new(journal: &Journal, mapped: Params, missing: bool) -> Self {
        Self {
            config: TestingConfig::returning(mapped),
            source: FakeSource,
            missing,
            journal: journal.clone(),
        }
    }
}

impl VideoGenerationProviders for FakeProviders {
    type Config = TestingConfig;
    type Source = FakeSource;

    fn video_config(&self, tag: &VideoConfigTag) -> Option<(&TestingConfig, &FakeSource)> {
        self.journal.push(Event::Lookup {
            config: tag.class_name().to_owned(),
        });
        if self.missing {
            None
        } else {
            Some((&self.config, &self.source))
        }
    }
}

pub(crate) struct FakeSource;

impl ClientSource for FakeSource {
    type Client = FakeTransport;

    fn client(&self, _request: ClientRequest<'_>) -> Result<FakeTransport, ClientError> {
        Ok(FakeTransport)
    }
}

pub(crate) struct FakeTransport;

impl VideoTransport for FakeTransport {
    fn send(
        &self,
        _request: VideoRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        future_ready(Ok(UpstreamResponse {
            status: 200,
            headers: HeaderMap::new(),
            body: Vec::new(),
        }))
    }
}

pub(crate) struct FakeHandler {
    pub(crate) journal: Journal,
    pub(crate) outcome: Outcome,
}

impl FakeHandler {
    fn new(journal: &Journal, outcome: Outcome) -> Self {
        Self {
            journal: journal.clone(),
            outcome,
        }
    }
}

impl<Config, Source> VideoGenerationHandling<Config, Source> for FakeHandler
where
    Config: VideoGenerationConfig,
    Source: ClientSource,
{
    fn handle<'call>(
        &'call self,
        environment: GenerationHandlerEnvironment<'call, '_, Config, Source>,
        request: GenerationHandlerRequest<'call, Source>,
        is_async: bool,
    ) -> PendingVideoGeneration<'call> {
        let GenerationHandlerEnvironment { logging, .. } = environment;
        self.journal.push(Event::Handled(Box::new(Handled {
            model: request.model.clone(),
            prompt: request.prompt.clone(),
            custom_llm_provider: request.custom_llm_provider.clone(),
            litellm_params: request.litellm_params.clone(),
            optional_params: request.optional_params.clone(),
            extra_headers: request.extra_headers.clone(),
            extra_body: request.extra_body.clone(),
            timeout: request.timeout,
            client: request.client.as_ref().map(|supplied| supplied.kind),
            is_async,
            logged: logged(logging),
        })));
        match self.outcome {
            Outcome::Answers => Box::pin(future_ready::<
                Result<VideoObject, VideoGenerationCallError>,
            >(Ok(video(VIDEO_ID)))),
            Outcome::FieldType => Box::pin(future_ready::<
                Result<VideoObject, VideoGenerationCallError>,
            >(Err(
                VideoGenerationCallError::FieldType {
                    field: "size",
                    expected: "a string",
                },
            ))),
            Outcome::NeverAnswers => {
                Box::pin(never_ready::<Result<VideoObject, VideoGenerationCallError>>())
            }
        }
    }
}

pub(crate) struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

pub(crate) struct LogEnv {
    redactor: Redactor,
    sink: Sink,
    now: DateTime<FixedOffset>,
}

impl LogEnv {
    fn new() -> Self {
        Self {
            redactor: redactor(),
            sink: Sink,
            now: moment(),
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
    env: LogEnv,
}

impl Fixture {
    fn new() -> Self {
        let init: LoggingInit = serde_json::from_str(
            r#"{"model":"video-model","call_type":"","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
        )
        .expect("the logging fixture is valid");
        let state = LoggingState::new(
            init,
            CorrelationSetup {
                context: &mut CorrelationContext::default(),
                settings: CorrelationSettings::default(),
            },
            &redactor(),
            || String::from("generated"),
            Duration::default,
        )
        .expect("the logging state builds");
        Self {
            state,
            env: LogEnv::new(),
        }
    }
}

// the resolution fixture the default-model inference runs against: an empty catalog and an empty environment
pub(crate) struct Resolver {
    catalog: ModelCatalog,
    names: BTreeSet<String>,
    environment: BTreeMap<String, String>,
}

impl Resolver {
    pub(crate) fn new() -> Self {
        Self {
            catalog: ModelCatalog::default(),
            names: BTreeSet::new(),
            environment: BTreeMap::new(),
        }
    }

    pub(crate) fn context(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.names,
            custom_providers: &self.names,
            environment: &self.environment,
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }
}

#[derive(Clone, Copy)]
enum Invocation {
    Sync,
    Async,
}

enum Invoked {
    Ready(Box<VideoObject>),
    Pending,
}

pub(crate) struct Bench {
    pub(crate) caller: VideoGenerationCaller<FakeProviders, FakeHandler>,
    pub(crate) fixture: Fixture,
    pub(crate) resolver: Resolver,
    pub(crate) journal: Journal,
}

impl Bench {
    pub(crate) fn new(outcome: Outcome) -> Self {
        Self::with(Params::new(), false, outcome)
    }

    pub(crate) fn with(mapped: Params, missing: bool, outcome: Outcome) -> Self {
        let journal = Journal::new();
        Self {
            caller: VideoGenerationCaller {
                providers: FakeProviders::new(&journal, mapped, missing),
                handler: FakeHandler::new(&journal, outcome),
                redactor: redactor(),
                request_timeout: REQUEST_TIMEOUT,
                drop_params: true,
            },
            fixture: Fixture::new(),
            resolver: Resolver::new(),
            journal,
        }
    }

    pub(crate) fn run(
        &mut self,
        request: VideoGenerationRequest,
    ) -> Result<Box<VideoObject>, VideoGenerationFailure> {
        self.run_with(request, None)
    }

    pub(crate) fn run_with(
        &mut self,
        request: VideoGenerationRequest,
        client: Option<&FakeTransport>,
    ) -> Result<Box<VideoObject>, VideoGenerationFailure> {
        let running = self.invoke(request, client, Invocation::Sync);
        settled(poll_once(running)).map(ready_object)
    }

    pub(crate) fn leaves_pending(
        &mut self,
        request: VideoGenerationRequest,
        client: Option<&FakeTransport>,
    ) -> bool {
        let running = self.invoke(request, client, Invocation::Sync);
        let work = settled(poll_once(running)).expect("the sync entry prepares the call");
        matches!(work, Invoked::Pending)
    }

    pub(crate) async fn async_run(
        &mut self,
        request: VideoGenerationRequest,
        client: Option<&FakeTransport>,
    ) -> Result<VideoObject, VideoGenerationFailure> {
        let work = self.invoke(request, client, Invocation::Async).await?;
        Ok(*ready_object(work))
    }

    async fn invoke(
        &mut self,
        request: VideoGenerationRequest,
        client: Option<&FakeTransport>,
        invocation: Invocation,
    ) -> Result<Invoked, VideoGenerationFailure> {
        let env = self.fixture.env.pre_post();
        let resolution = self.resolver.context();
        let environment = GenerationCallEnvironment {
            logging: &mut self.fixture.state,
            env: &env,
        };
        let work = match invocation {
            Invocation::Sync => {
                self.caller
                    .video_generation(request, client, &resolution, environment)
            }
            Invocation::Async => self
                .caller
                .avideo_generation(request, client, &resolution, environment)
                .await
                .map(|object| VideoGenerationWork::Ready(Box::new(object))),
        }?;
        Ok(match work {
            VideoGenerationWork::Ready(object) => Invoked::Ready(object),
            VideoGenerationWork::Pending(_) => Invoked::Pending,
        })
    }
}

fn settled<Output>(polled: Polled<Output>) -> Output {
    match polled {
        Polled::Ready(output) => output,
        Polled::StillWaiting => panic!("the sync fixture must settle on its first poll"),
    }
}

fn ready_object(work: Invoked) -> Box<VideoObject> {
    match work {
        Invoked::Ready(object) => object,
        Invoked::Pending => {
            panic!("the sync entry left a pending handler for an async caller")
        }
    }
}

pub(crate) fn failed(
    work: Result<Box<VideoObject>, VideoGenerationFailure>,
) -> VideoGenerationFailure {
    work.unwrap_err()
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

pub(crate) fn request() -> VideoGenerationRequest {
    VideoGenerationRequest::new(String::from("a prompt"))
}

pub(crate) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(crate) fn params(pairs: &[(&str, Json)]) -> Params {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

pub(crate) fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}
