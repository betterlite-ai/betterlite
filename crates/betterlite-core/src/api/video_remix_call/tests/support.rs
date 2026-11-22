use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::io;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::RequestBody;
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
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use betterlite_openai::videos::VideoOptions;
use betterlite_openai::videos::config::{VideoCredentials, complete_url, validate_environment};
use betterlite_openai::videos::list::{list_request, list_response};
use betterlite_openai::videos::remix::{remix_request, remix_response};
use betterlite_openai::videos::reply::VideoObject as OpenAiVideoObject;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use http::HeaderMap as HttpHeaders;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use crate::api::video_list_call::VideoListCall;
use crate::api::video_records::VideoObject;
use crate::api::video_remix_call::config::VideoConfigLookup;
use crate::api::video_remix_call::{VideoRemixCall, VideoRemixDeps, avideo_remix, video_remix};
use crate::api::video_remix_list_transport::VideoRemixListTransport;
use crate::api::video_remix_list_transport::client::{
    VideoAsyncClient, VideoClient, VideoClientSource, VideoRequest, VideoSyncClient,
};
use crate::api::video_remix_list_transport::config::VideoRemixListConfig;
use crate::api::video_remix_list_transport::error::{ConfigFailure, ErrorHeaders};
use crate::api::video_remix_list_transport::records::{
    Headers, Params, VideoEnvironment, VideoListQuery, VideoReply, VideoRequestContext,
};
use crate::api::video_selection::VideoConfigTag;

pub(crate) struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

// The fixture's logging environment: one redactor and a clock the pre-call records
pub(crate) struct LogEnv {
    redactor: Redactor,
    sink: Sink,
    now: DateTime<FixedOffset>,
}

impl LogEnv {
    fn new() -> Self {
        Self {
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds"),
            sink: Sink,
            now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00")
                .expect("the clock parses"),
        }
    }

    pub(crate) fn pre_post(&self) -> PrePostEnv<'_> {
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

    pub(crate) fn redactor(&self) -> &Redactor {
        &self.redactor
    }
}

pub(crate) struct Fixture {
    pub(crate) state: LoggingState,
    pub(crate) env: LogEnv,
}

impl Fixture {
    pub(crate) fn new() -> Self {
        let init: LoggingInit = serde_json::from_str(
            r#"{"model":"video","call_type":"","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
        )
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
            env: LogEnv::new(),
        }
    }
}

// The provider reply one wire answers with
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
}

// One client that records every request and answers the same reply
#[derive(Clone)]
pub(crate) struct Wire {
    reply: Reply,
    requests: Arc<Mutex<Vec<VideoRequest>>>,
}

impl Wire {
    pub(crate) fn replying(status: u16, body: &str) -> Self {
        Self {
            reply: Reply::json(status, body),
            requests: Arc::default(),
        }
    }

    pub(crate) fn requests(&self) -> MutexGuard<'_, Vec<VideoRequest>> {
        self.requests.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn answer(&self, request: VideoRequest) -> Result<UpstreamResponse, TransportError> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
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

type PoolCall = (ProviderId, Option<bool>);

// The B173 pool for the tests: one wire, and the calls the handlers made to choose a client
#[derive(Clone)]
pub(crate) struct Source {
    wire: Wire,
    sync_calls: Arc<Mutex<Vec<Option<bool>>>>,
    async_calls: Arc<Mutex<Vec<PoolCall>>>,
}

impl Source {
    fn new(wire: Wire) -> Self {
        Self {
            wire,
            sync_calls: Arc::default(),
            async_calls: Arc::default(),
        }
    }

    pub(crate) fn sync_calls(&self) -> Vec<Option<bool>> {
        self.sync_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn async_calls(&self) -> Vec<PoolCall> {
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
    type Error = io::Error;

    fn sync_client(&self, ssl_verify: Option<bool>) -> Result<Wire, io::Error> {
        record(&self.sync_calls, ssl_verify);
        Ok(self.wire.clone())
    }

    fn async_client(
        &self,
        provider: ProviderId,
        ssl_verify: Option<bool>,
    ) -> Result<Wire, io::Error> {
        record(&self.async_calls, (provider, ssl_verify));
        Ok(self.wire.clone())
    }
}

// The OpenAI video config over B525's free functions: the adapter the call layer hands the transport
pub(crate) struct OpenAiConfig;

fn options(params: &Params) -> VideoOptions {
    serde_json::from_value(serde_json::to_value(params).expect("params serialize"))
        .expect("params are video options")
}

fn params_from(options: VideoOptions) -> Params {
    serde_json::from_value(serde_json::to_value(options).expect("options serialize"))
        .expect("video options are params")
}

fn core_video(video: &OpenAiVideoObject) -> VideoObject {
    serde_json::from_value(serde_json::to_value(video).expect("the openai video serializes"))
        .expect("the openai video is a core video")
}

fn params_key(params: Option<&Params>) -> Option<SecretString> {
    match params?.get("api_key")? {
        Json::Text(text) => Some(SecretString::from(text.clone())),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

fn input_headers(headers: &Headers) -> BTreeMap<String, SecretString> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), SecretString::from(value.clone())))
        .collect()
}

impl VideoRemixListConfig for OpenAiConfig {
    fn validate_environment(&self, request: VideoEnvironment<'_>) -> Headers {
        let key = match request.api_key {
            Some(key) => Some(key.clone()),
            None => params_key(request.litellm_params),
        };
        let credentials = VideoCredentials {
            api_key: key,
            ..VideoCredentials::default()
        };
        validate_environment(input_headers(request.headers), &credentials)
            .into_iter()
            .map(|(name, value)| (name, value.expose_secret().to_owned()))
            .collect()
    }

    fn get_complete_url(
        &self,
        _model: &str,
        api_base: Option<&str>,
        _litellm_params: &Params,
    ) -> Result<String, ConfigFailure> {
        Ok(complete_url(api_base))
    }

    fn transform_video_remix_request(
        &self,
        video_id: &str,
        prompt: &str,
        extra_body: Option<&Params>,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), ConfigFailure> {
        let extra = extra_body.map_or_else(VideoOptions::new, options);
        remix_request(video_id, prompt, context.api_base, &extra)
            .map(|(url, body)| (url, params_from(body)))
            .map_err(|error| ConfigFailure {
                source: Box::new(error),
            })
    }

    fn transform_video_remix_response(
        &self,
        reply: VideoReply<'_>,
    ) -> Result<VideoObject, ConfigFailure> {
        remix_response(&reply.raw_response.body, reply.custom_llm_provider)
            .map(|video| core_video(&video))
            .map_err(|error| ConfigFailure {
                source: Box::new(error),
            })
    }

    fn transform_video_list_request(
        &self,
        query: VideoListQuery<'_>,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), ConfigFailure> {
        let extra = query.extra_query.map_or_else(VideoOptions::new, options);
        let (url, params) = list_request(
            context.api_base,
            query.after,
            query.limit,
            query.order,
            &extra,
        );
        Ok((url, params_from(params)))
    }

    fn transform_video_list_response(
        &self,
        reply: VideoReply<'_>,
    ) -> Result<Params, ConfigFailure> {
        let list = list_response(&reply.raw_response.body, reply.custom_llm_provider).map_err(
            |error| ConfigFailure {
                source: Box::new(error),
            },
        )?;
        let encoded = serde_json::to_value(&list).expect("the openai list serializes");
        Ok(serde_json::from_value(encoded).expect("the openai list is params"))
    }

    fn get_error_class(
        &self,
        message: &str,
        status_code: u16,
        _headers: ErrorHeaders,
    ) -> ProviderError {
        ProviderError::from(ProviderErrorParts {
            status_code,
            message: message.to_owned(),
            headers: None,
            request: None,
            response: None,
            body: None,
            status_code_is_synthesized: false,
        })
    }
}

// The lookup the call layer is given: only the OpenAI tag resolves
pub(crate) struct Lookup {
    openai: OpenAiConfig,
}

impl Lookup {
    fn new() -> Self {
        Self {
            openai: OpenAiConfig,
        }
    }
}

impl VideoConfigLookup for Lookup {
    fn video_config(&self, tag: &VideoConfigTag) -> Option<&dyn VideoRemixListConfig> {
        match tag {
            VideoConfigTag::OpenAi => Some(&self.openai),
            VideoConfigTag::Azure
            | VideoConfigTag::Gemini
            | VideoConfigTag::VertexAi
            | VideoConfigTag::RunwayMl
            | VideoConfigTag::FalAi
            | VideoConfigTag::HostedVllm(_) => None,
        }
    }
}

// One case's logging object, config, pool and transport, all sharing one wire
pub(crate) struct Bench {
    pub(crate) fixture: Fixture,
    pub(crate) configs: Lookup,
    pub(crate) source: Source,
    pub(crate) transport: VideoRemixListTransport<Source>,
}

impl Bench {
    pub(crate) fn new(status: u16, body: &str) -> Self {
        let wire = Wire::replying(status, body);
        let source = Source::new(wire);
        Self {
            fixture: Fixture::new(),
            configs: Lookup::new(),
            transport: VideoRemixListTransport {
                source: source.clone(),
            },
            source,
        }
    }

    pub(crate) fn requests(&self) -> MutexGuard<'_, Vec<VideoRequest>> {
        self.source.wire.requests()
    }
}

pub(crate) const VIDEO_BODY: &str =
    r#"{"id":"video_123","object":"video","status":"in_progress","seconds":"4"}"#;
pub(crate) const LIST_BODY: &str = r#"{"object":"list","data":[{"id":"video_1","object":"video","status":"queued"}],"has_more":false}"#;

// The two entries, called with the bench's own logging object and pool
pub(crate) fn run_remix(
    bench: &mut Bench,
    call: &VideoRemixCall,
    request_timeout_seconds: f64,
) -> Result<VideoObject, crate::api::video_remix_call::error::VideoCallFailure> {
    let env = bench.fixture.env.pre_post();
    let deps = VideoRemixDeps {
        configs: &bench.configs,
        logging: &mut bench.fixture.state,
        env: &env,
        redactor: bench.fixture.env.redactor(),
        request_timeout_seconds,
    };
    video_remix(&bench.transport, deps, call, None)
}

pub(crate) fn run_remix_with_client(
    bench: &mut Bench,
    call: &VideoRemixCall,
    client: VideoClient<'_, Source>,
) -> Result<VideoObject, crate::api::video_remix_call::error::VideoCallFailure> {
    let env = bench.fixture.env.pre_post();
    let deps = VideoRemixDeps {
        configs: &bench.configs,
        logging: &mut bench.fixture.state,
        env: &env,
        redactor: bench.fixture.env.redactor(),
        request_timeout_seconds: 6000.0,
    };
    video_remix(&bench.transport, deps, call, Some(client))
}

pub(crate) async fn run_avideo_remix(
    bench: &mut Bench,
    call: &VideoRemixCall,
    request_timeout_seconds: f64,
) -> Result<VideoObject, crate::api::video_remix_call::error::VideoCallFailure> {
    let env = bench.fixture.env.pre_post();
    let deps = VideoRemixDeps {
        configs: &bench.configs,
        logging: &mut bench.fixture.state,
        env: &env,
        redactor: bench.fixture.env.redactor(),
        request_timeout_seconds,
    };
    crate::api::video_remix_call::avideo_remix(&bench.transport, deps, call, None).await
}

pub(crate) async fn run_avideo_remix_with_client(
    bench: &mut Bench,
    call: &VideoRemixCall,
    client: VideoClient<'_, Source>,
) -> Result<VideoObject, crate::api::video_remix_call::error::VideoCallFailure> {
    let env = bench.fixture.env.pre_post();
    let deps = VideoRemixDeps {
        configs: &bench.configs,
        logging: &mut bench.fixture.state,
        env: &env,
        redactor: bench.fixture.env.redactor(),
        request_timeout_seconds: 6000.0,
    };
    crate::api::video_remix_call::avideo_remix(&bench.transport, deps, call, Some(client)).await
}

// The resolution fixture avideo_list infers from an empty model with
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

pub(crate) fn run_list(
    bench: &mut Bench,
    call: &VideoListCall,
    request_timeout_seconds: f64,
) -> Result<
    crate::api::video_list_call::VideoListReply,
    crate::api::video_remix_call::error::VideoCallFailure,
> {
    let env = bench.fixture.env.pre_post();
    let deps = crate::api::video_list_call::VideoListDeps {
        configs: &bench.configs,
        logging: &mut bench.fixture.state,
        env: &env,
        redactor: bench.fixture.env.redactor(),
        request_timeout_seconds,
    };
    crate::api::video_list_call::video_list(&bench.transport, deps, call, None)
}

pub(crate) async fn run_avideo_list(
    bench: &mut Bench,
    resolver: &Resolver,
    call: &VideoListCall,
    request_timeout_seconds: f64,
) -> Result<
    crate::api::video_list_call::VideoListReply,
    crate::api::video_remix_call::error::VideoCallFailure,
> {
    let env = bench.fixture.env.pre_post();
    let deps = crate::api::video_list_call::VideoListDeps {
        configs: &bench.configs,
        logging: &mut bench.fixture.state,
        env: &env,
        redactor: bench.fixture.env.redactor(),
        request_timeout_seconds,
    };
    let resolution = resolver.context();
    crate::api::video_list_call::avideo_list(&bench.transport, deps, &resolution, call, None).await
}

pub(crate) fn run_list_with_client(
    bench: &mut Bench,
    call: &VideoListCall,
    client: VideoClient<'_, Source>,
) -> Result<
    crate::api::video_list_call::VideoListReply,
    crate::api::video_remix_call::error::VideoCallFailure,
> {
    let env = bench.fixture.env.pre_post();
    let deps = crate::api::video_list_call::VideoListDeps {
        configs: &bench.configs,
        logging: &mut bench.fixture.state,
        env: &env,
        redactor: bench.fixture.env.redactor(),
        request_timeout_seconds: 6000.0,
    };
    crate::api::video_list_call::video_list(&bench.transport, deps, call, Some(client))
}

pub(crate) async fn run_avideo_list_with_client(
    bench: &mut Bench,
    resolver: &Resolver,
    call: &VideoListCall,
    client: VideoClient<'_, Source>,
) -> Result<
    crate::api::video_list_call::VideoListReply,
    crate::api::video_remix_call::error::VideoCallFailure,
> {
    let env = bench.fixture.env.pre_post();
    let deps = crate::api::video_list_call::VideoListDeps {
        configs: &bench.configs,
        logging: &mut bench.fixture.state,
        env: &env,
        redactor: bench.fixture.env.redactor(),
        request_timeout_seconds: 6000.0,
    };
    let resolution = resolver.context();
    crate::api::video_list_call::avideo_list(
        &bench.transport,
        deps,
        &resolution,
        call,
        Some(client),
    )
    .await
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

pub(crate) fn request_query(request: &VideoRequest, name: &str) -> Option<String> {
    request
        .query
        .iter()
        .find(|(key, _value)| key == name)
        .map(|(_key, value)| value.clone())
}

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the JSON fixture parses")
}

pub(crate) fn encoded_id(video_id: &str, provider: &str) -> String {
    betterlite_openai::openai_video_identifiers::ids::encode_video_id_with_provider(
        video_id, provider, None,
    )
}

pub(crate) fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

pub(crate) fn headers(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn params(text: &str) -> Params {
    serde_json::from_str(text).expect("the params fixture is valid")
}
