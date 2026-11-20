use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse, base_builder};
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
use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use http::HeaderValue;
use http::header::HeaderMap as HttpHeaders;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use crate::api::video_records::VideoObject;
use crate::api::videos_http_transport::client::{
    ClientError, ClientKind, ClientRequest, ClientSource, SuppliedClient, VideoDeps, VideoRequest,
    VideoTransport,
};
use crate::api::videos_http_transport::content::{VideoContentCall, video_content_handler};
use crate::api::videos_http_transport::error::VideoTransportError;
use crate::api::videos_http_transport::generation::{
    LogContext, VideoGenerationCall, video_generation_handler,
};
use crate::api::videos_http_transport::video_config::{
    ErrorFacts, Headers, Params, RequestFile, RequestFiles, VideoEnvironment, VideoHttpConfig,
    VideoLogging, VideoReply, VideoRequestContext,
};

pub(crate) const MODEL: &str = "video-model";
pub(crate) const PROMPT: &str = "a cat on a skateboard";
pub(crate) const PROVIDER: &str = "openai";
pub(crate) const BASE: &str = "http://mock.test/v1";

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
    pub(crate) fn new() -> Self {
        let init: LoggingInit = serde_json::from_str(
            r#"{"model":"video-model","call_type":"video_generation","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
        )
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

    pub(crate) async fn generate<C, S>(
        &mut self,
        deps: &VideoDeps<'_, C, S>,
        call: &VideoGenerationCall<'_>,
        events: &Events,
        is_async: bool,
    ) -> Result<VideoObject, VideoTransportError<C::Error>>
    where
        C: VideoHttpConfig,
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
        let mut context = LogContext {
            state: &mut self.state,
            env: &env,
        };
        video_generation_handler(deps, call, &mut context, is_async).await
    }

    pub(crate) async fn content<C, S>(
        &mut self,
        deps: &VideoDeps<'_, C, S>,
        call: &VideoContentCall<'_>,
        is_async: bool,
    ) -> Result<Vec<u8>, VideoTransportError<C::Error>>
    where
        C: VideoHttpConfig,
        S: ClientSource,
    {
        video_content_handler(deps, call, &mut self.state, is_async).await
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
    pub(crate) fn new(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: HttpHeaders::new(),
            body: body.as_bytes().to_vec(),
        }
    }

    pub(crate) fn with_header(mut self, name: &'static str, value: &str) -> Self {
        self.headers.insert(
            name,
            HeaderValue::from_str(value).expect("the header value"),
        );
        self
    }

    pub(crate) fn bytes(status: u16, body: Vec<u8>) -> Self {
        Self {
            status,
            headers: HttpHeaders::new(),
            body,
        }
    }

    pub(crate) fn with_raw_header(mut self, name: &'static str, value: &[u8]) -> Self {
        self.headers.insert(
            name,
            HeaderValue::from_bytes(value).expect("the header bytes"),
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

    pub(crate) fn failing(error: TransportError, events: &Events) -> Self {
        let wire = Self::replying(Reply::new(200, "{}"), events);
        *wire.failure.lock().unwrap_or_else(PoisonError::into_inner) = Some(error);
        wire
    }

    pub(crate) fn requests(&self) -> MutexGuard<'_, Vec<VideoRequest>> {
        self.requests.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl VideoTransport for Wire {
    fn send(
        &self,
        request: VideoRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.events.push("send");
        let failure = self
            .failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        if let Some(error) = failure {
            return ready(Err(error));
        }
        ready(Ok(UpstreamResponse {
            status: self.reply.status,
            headers: self.reply.headers.clone(),
            body: self.reply.body.clone(),
        }))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ClientRequestRecord {
    pub(crate) kind: ClientKind,
    pub(crate) ssl_verify: Option<bool>,
    pub(crate) provider: String,
}

pub(crate) struct Source {
    wire: Wire,
    calls: Arc<Mutex<Vec<ClientRequestRecord>>>,
}

impl Source {
    pub(crate) fn new(wire: Wire) -> Self {
        Self {
            wire,
            calls: Arc::default(),
        }
    }

    pub(crate) fn calls(&self) -> Vec<ClientRequestRecord> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ClientSource for Source {
    type Client = Wire;

    fn client(&self, request: ClientRequest<'_>) -> Result<Wire, ClientError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ClientRequestRecord {
                kind: request.kind,
                ssl_verify: request.ssl_verify,
                provider: request.custom_llm_provider.to_owned(),
            });
        Ok(self.wire.clone())
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

    fn client(&self, _request: ClientRequest<'_>) -> Result<ProxiedClient, ClientError> {
        Ok(self.client.clone())
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum TestError {
    #[error("the transform refused")]
    Transform,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Environment {
    pub(crate) headers: Headers,
    pub(crate) model: String,
    pub(crate) api_key: Option<String>,
    pub(crate) litellm_params: Option<Params>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CreateReply {
    pub(crate) model: String,
    pub(crate) provider: Option<String>,
    pub(crate) request_data: Option<Params>,
}

// The transform a case asks to fail, so the config keeps one list instead of four switches
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    CreateRequest,
    CreateResponse,
    ContentRequest,
    ContentResponse,
}

// The provider's side of both handlers, with every answer a case can vary and every argument it can check
pub(crate) struct RecordingConfig {
    pub(crate) base: String,
    pub(crate) multipart: bool,
    pub(crate) create_data: Params,
    pub(crate) create_files: RequestFiles,
    pub(crate) content_data: Params,
    pub(crate) failures: Vec<Failure>,
    pub(crate) content_error_status: Option<u16>,
    pub(crate) environments: Mutex<Vec<Environment>>,
    pub(crate) urls: Mutex<Vec<(String, Option<String>)>>,
    pub(crate) create_contexts: Mutex<Vec<(String, Headers)>>,
    pub(crate) content_contexts: Mutex<Vec<(String, Option<String>)>>,
    pub(crate) create_replies: Mutex<Vec<CreateReply>>,
    pub(crate) content_replies: Mutex<Vec<(u16, Vec<u8>)>>,
    pub(crate) transforms: Mutex<Vec<&'static str>>,
    pub(crate) errors: Mutex<Vec<(String, u16, Headers)>>,
}

impl RecordingConfig {
    pub(crate) fn new(base: &str) -> Self {
        Self {
            base: base.to_owned(),
            multipart: false,
            create_data: Params::new(),
            create_files: Vec::new(),
            content_data: Params::new(),
            failures: Vec::new(),
            content_error_status: None,
            environments: Mutex::default(),
            urls: Mutex::default(),
            create_contexts: Mutex::default(),
            content_contexts: Mutex::default(),
            create_replies: Mutex::default(),
            content_replies: Mutex::default(),
            transforms: Mutex::default(),
            errors: Mutex::default(),
        }
    }

    fn record(&self, name: &'static str) {
        self.transforms
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(name);
    }

    fn content_response(
        &self,
        raw_response: &UpstreamResponse,
        logging_obj: &mut VideoLogging,
        name: &'static str,
        mark: &'static str,
    ) -> Result<Vec<u8>, TestError> {
        self.record(name);
        self.content_replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((raw_response.status, raw_response.body.clone()));
        if self.failures.contains(&Failure::ContentResponse) {
            logging_obj
                .model_call_details
                .insert("content_failed_mark".to_owned(), Json::Bool(true));
            return Err(TestError::Transform);
        }
        logging_obj
            .model_call_details
            .insert("content_mark".to_owned(), Json::Text(mark.to_owned()));
        Ok(raw_response.body.clone())
    }

    pub(crate) fn transforms(&self) -> Vec<&'static str> {
        self.transforms
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn environments(&self) -> Vec<Environment> {
        self.environments
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn errors(&self) -> Vec<(String, u16, Headers)> {
        self.errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn content_replies(&self) -> Vec<(u16, Vec<u8>)> {
        self.content_replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn create_replies(&self) -> Vec<CreateReply> {
        self.create_replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn urls(&self) -> Vec<(String, Option<String>)> {
        self.urls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn create_contexts(&self) -> Vec<(String, Headers)> {
        self.create_contexts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn content_contexts(&self) -> Vec<(String, Option<String>)> {
        self.content_contexts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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
        headers.insert("Authorization".to_owned(), "Bearer sk-provider".to_owned());
        headers.insert("Content-Type".to_owned(), "application/json".to_owned());
        headers
    }

    fn get_complete_url(
        &self,
        model: &str,
        api_base: Option<&str>,
        _litellm_params: &Params,
    ) -> Result<String, TestError> {
        self.urls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((model.to_owned(), api_base.map(str::to_owned)));
        Ok(self.base.clone())
    }

    fn use_multipart_form_data(&self) -> bool {
        self.multipart
    }

    fn transform_video_create_request(
        &self,
        _model: &str,
        _prompt: &str,
        _optional_params: &Params,
        context: VideoRequestContext<'_>,
    ) -> Result<(Params, RequestFiles, String), TestError> {
        self.record("create_request");
        self.create_contexts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((context.api_base.to_owned(), context.headers.clone()));
        if self.failures.contains(&Failure::CreateRequest) {
            return Err(TestError::Transform);
        }
        Ok((
            self.create_data.clone(),
            self.create_files.clone(),
            format!("{}/submit", context.api_base),
        ))
    }

    fn transform_video_create_response(
        &self,
        model: &str,
        reply: VideoReply<'_>,
        request_data: Option<&Params>,
    ) -> Result<VideoObject, TestError> {
        self.record("create_response");
        self.create_replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(CreateReply {
                model: model.to_owned(),
                provider: reply.custom_llm_provider.map(str::to_owned),
                request_data: request_data.cloned(),
            });
        if self.failures.contains(&Failure::CreateResponse) {
            reply
                .logging_obj
                .model_call_details
                .insert("create_failed_mark".to_owned(), Json::Bool(true));
            return Err(TestError::Transform);
        }
        reply
            .logging_obj
            .model_call_details
            .insert("response_mark".to_owned(), Json::Text("created".to_owned()));
        Ok(VideoObject::new("video-1".to_owned(), "queued".to_owned()))
    }

    fn transform_video_content_request(
        &self,
        video_id: &str,
        variant: Option<&str>,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), TestError> {
        self.record("content_request");
        self.content_contexts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((context.api_base.to_owned(), variant.map(str::to_owned)));
        if self.failures.contains(&Failure::ContentRequest) {
            return Err(TestError::Transform);
        }
        Ok((
            format!("{}/contents/{video_id}", context.api_base),
            self.content_data.clone(),
        ))
    }

    fn transform_video_content_response(
        &self,
        raw_response: &UpstreamResponse,
        logging_obj: &mut VideoLogging,
    ) -> Result<Vec<u8>, TestError> {
        self.content_response(raw_response, logging_obj, "content_sync", "sync")
    }

    fn async_transform_video_content_response(
        &self,
        raw_response: &UpstreamResponse,
        logging_obj: &mut VideoLogging,
    ) -> impl Future<Output = Result<Vec<u8>, TestError>> + Send {
        ready(self.content_response(raw_response, logging_obj, "content_async", "async"))
    }

    fn error_facts(&self, error: &TestError) -> ErrorFacts {
        ErrorFacts {
            message: error.to_string(),
            status_code: self.content_error_status,
            headers: headers(&[("x-transform", "raised")]),
        }
    }

    fn get_error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: &Headers,
    ) -> ProviderError {
        self.errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((error_message.to_owned(), status_code, headers.clone()));
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

pub(crate) struct Bench {
    pub(crate) events: Events,
    pub(crate) config: RecordingConfig,
    pub(crate) wire: Wire,
    pub(crate) source: Source,
    pub(crate) fixture: Fixture,
}

impl Bench {
    pub(crate) fn new() -> Self {
        Self::with_reply(Reply::new(200, "video-bytes"))
    }

    pub(crate) fn with_reply(answer: Reply) -> Self {
        let events = Events::new();
        let wire = Wire::replying(answer, &events);
        Self {
            config: RecordingConfig::new(BASE),
            source: Source::new(wire.clone()),
            wire,
            events,
            fixture: Fixture::new(),
        }
    }
}

pub(crate) const fn video_deps<'deps, C, S: ClientSource>(
    config: &'deps C,
    source: &'deps S,
    supplied: Option<SuppliedClient<'deps, S::Client>>,
) -> VideoDeps<'deps, C, S> {
    VideoDeps {
        config,
        source,
        supplied,
    }
}

pub(crate) fn params(json: &str) -> Params {
    serde_json::from_str(json).expect("the params fixture is valid JSON")
}

pub(crate) fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

pub(crate) fn request_file(
    field: &str,
    filename: Option<&str>,
    content_type: Option<&str>,
) -> RequestFile {
    RequestFile {
        field: field.to_owned(),
        filename: filename.map(str::to_owned),
        content: b"file-bytes".to_vec(),
        content_type: content_type.map(str::to_owned),
    }
}

pub(crate) const fn generation_call<'call>(
    optional: &'call Params,
    litellm: &'call Params,
) -> VideoGenerationCall<'call> {
    VideoGenerationCall {
        model: MODEL,
        prompt: PROMPT,
        optional_params: optional,
        custom_llm_provider: PROVIDER,
        litellm_params: litellm,
        extra_headers: None,
        timeout: Duration::from_secs(30),
        api_key: None,
    }
}

pub(crate) const fn content_call<'call>(
    video_id: &'call str,
    litellm: &'call Params,
) -> VideoContentCall<'call> {
    let timeout = Duration::from_secs(30);
    VideoContentCall {
        video_id,
        custom_llm_provider: PROVIDER,
        litellm_params: litellm,
        variant: None,
        extra_headers: None,
        api_key: None,
        timeout,
    }
}

pub(crate) fn body_json(request: &VideoRequest) -> Json {
    let RequestBody::Json(body) = &request.body else {
        panic!("the request is not JSON");
    };
    serde_json::from_slice(body).expect("the request body is JSON")
}

pub(crate) fn body_parts(request: &VideoRequest) -> (&str, &[MultipartPart]) {
    let RequestBody::Multipart { boundary, parts } = &request.body else {
        panic!("the request is not multipart");
    };
    (boundary.as_str(), parts.as_slice())
}

pub(crate) fn text_parts(request: &VideoRequest) -> Vec<(String, String)> {
    let (_, parts) = body_parts(request);
    parts
        .iter()
        .map(|part| {
            (
                part.name.clone(),
                String::from_utf8(part.data.clone()).expect("the part is text"),
            )
        })
        .collect()
}

pub(crate) fn proxied_client(limits: TransportLimits) -> ProxiedClient {
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
