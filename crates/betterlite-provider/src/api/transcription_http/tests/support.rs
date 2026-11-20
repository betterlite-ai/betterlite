use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_core::api::vector_store_http_write::error::ClientError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::RequestBody;
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
use betterlite_openai::api::transcription_contract::{
    AudioTranscriptionData, AudioTranscriptionRequestData, TranscriptionResponse,
};
use betterlite_openai::openai_base_config::Object;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use http::header::HeaderMap as HttpHeaders;

use crate::api::transcription_contract::{
    AudioFile, AudioTranscriptionConfig, TranscriptionContractError,
};
use crate::api::transcription_http::config::{
    ClientRequest, ClientSource, ConfigFailure, Headers, TranscriptionHttpConfig, UrlInput,
    Validation,
};
use crate::api::transcription_http::error::TranscriptionHttpError;
use crate::api::transcription_http::reply::TranscriptionReply;
use crate::api::transcription_http::wire::{
    ClientKind, SuppliedClient, TranscriptionBody, TranscriptionRequest, TranscriptionTransport,
};
use crate::api::transcription_http::{
    LogContext, TranscriptionCall, TranscriptionDeps, async_audio_transcriptions,
    audio_transcriptions,
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
            r#"{{"model":"transcription","call_type":"{call_type}","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}}"#
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

    // The call runs through the same logging state a dispatched request would carry
    pub(crate) async fn run<C, S>(
        &mut self,
        events: &Events,
        deps: &TranscriptionDeps<'_, C, S>,
        call: &TranscriptionCall<'_>,
        mode: Entry,
    ) -> Result<TranscriptionReply, TranscriptionHttpError>
    where
        C: TranscriptionHttpConfig,
        S: ClientSource,
    {
        let recorded = events.clone();
        let logger = move |_details: &Json| -> Result<(), PrePostError> {
            recorded.push("log");
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
        let mut log = LogContext {
            state: &mut self.state,
            env: &env,
        };
        match mode {
            Entry::Audio => audio_transcriptions(deps, call, &mut log).await,
            Entry::Async => async_audio_transcriptions(deps, call, &mut log).await,
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

    pub(crate) fn complete_input(&self) -> Option<&Json> {
        self.logged("complete_input_dict")
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Entry {
    Audio,
    Async,
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
    requests: Arc<Mutex<Vec<TranscriptionRequest>>>,
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
        let wire = Self::replying(Reply::json(200, "{}"), events);
        *wire.failure.lock().unwrap_or_else(PoisonError::into_inner) = Some(error);
        wire
    }

    pub(crate) fn requests(&self) -> MutexGuard<'_, Vec<TranscriptionRequest>> {
        self.requests.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl TranscriptionTransport for Wire {
    async fn send(
        &self,
        request: TranscriptionRequest,
    ) -> Result<UpstreamResponse, TransportError> {
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
            return Err(error);
        }
        Ok(UpstreamResponse {
            status: self.reply.status,
            headers: self.reply.headers.clone(),
            body: self.reply.body.clone(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ClientCall {
    pub(crate) ssl_verify: Option<bool>,
    pub(crate) shared_session: bool,
}

pub(crate) struct Source {
    wire: Wire,
    calls: Arc<Mutex<Vec<ClientCall>>>,
}

impl Source {
    pub(crate) fn new(wire: Wire) -> Self {
        Self {
            wire,
            calls: Arc::default(),
        }
    }

    pub(crate) fn calls(&self) -> Vec<ClientCall> {
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
            .push(ClientCall {
                ssl_verify: request.ssl_verify,
                shared_session: request.shared_session.is_some(),
            });
        Ok(self.wire.clone())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum StageFailure {
    #[default]
    None,
    Validate,
    Url,
    Transform,
    Response,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ValidationRecord {
    pub(crate) api_key: Option<String>,
    pub(crate) model: String,
    pub(crate) messages: usize,
    pub(crate) headers: Headers,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UrlRecord {
    pub(crate) api_base: Option<String>,
    pub(crate) api_key: Option<String>,
    pub(crate) model: String,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Calls {
    pub(crate) validations: Vec<ValidationRecord>,
    pub(crate) urls: Vec<UrlRecord>,
    pub(crate) transforms: usize,
    pub(crate) responses: usize,
}

pub(crate) struct RecordingConfig {
    pub(crate) data: AudioTranscriptionRequestData,
    pub(crate) subtitles: bool,
    pub(crate) response: TranscriptionResponse,
    pub(crate) failure: StageFailure,
    pub(crate) events: Events,
    pub(crate) calls: Mutex<Calls>,
}

impl RecordingConfig {
    pub(crate) fn new(data: AudioTranscriptionRequestData, events: &Events) -> Self {
        Self {
            data,
            subtitles: false,
            response: TranscriptionResponse {
                text: Some(String::from("hi")),
                extra: Object::new(),
            },
            failure: StageFailure::None,
            events: events.clone(),
            calls: Mutex::default(),
        }
    }

    pub(crate) fn calls(&self) -> Calls {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl AudioTranscriptionConfig for RecordingConfig {
    fn supported_openai_params(&self, _model: &str) -> Vec<String> {
        Vec::new()
    }

    fn transform_audio_request(
        &self,
        _model: &str,
        _audio_file: &AudioFile<'_>,
        _optional_params: &Object,
        _litellm_params: &Object,
    ) -> Result<AudioTranscriptionRequestData, TranscriptionContractError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .transforms += 1;
        self.events.push("transform-request");
        if self.failure == StageFailure::Transform {
            return Err(TranscriptionContractError::MissingRequestTransform);
        }
        Ok(self.data.clone())
    }

    fn supports_subtitle_synthesis(&self) -> bool {
        self.subtitles
    }

    fn complete_url(
        &self,
        api_base: Option<&str>,
        _model: &str,
        _optional_params: &Object,
        _litellm_params: &Object,
    ) -> String {
        format!("{}/audio/transcriptions", api_base.unwrap_or_default())
    }

    fn transform_audio_response(
        &self,
        _raw_response: &UpstreamResponse,
    ) -> Result<TranscriptionResponse, TranscriptionContractError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .responses += 1;
        self.events.push("transform-response");
        if self.failure == StageFailure::Response {
            return Err(TranscriptionContractError::UnexpectedResponseTransform);
        }
        Ok(self.response.clone())
    }
}

impl TranscriptionHttpConfig for RecordingConfig {
    fn validate_environment(&self, input: &Validation<'_>) -> Result<Headers, ConfigFailure> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .validations
            .push(ValidationRecord {
                api_key: input.api_key.map(str::to_owned),
                model: input.model.to_owned(),
                messages: input.messages.len(),
                headers: input.headers.clone(),
            });
        if self.failure == StageFailure::Validate {
            return Err(ConfigFailure {
                message: String::from("the fixture config refused the call"),
            });
        }
        let mut headers = input.headers.clone();
        if let Some(key) = input.api_key {
            headers.insert(String::from("Authorization"), format!("Bearer {key}"));
        }
        Ok(headers)
    }

    fn complete_transcription_url(&self, input: &UrlInput<'_>) -> Result<String, ConfigFailure> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .urls
            .push(UrlRecord {
                api_base: input.api_base.map(str::to_owned),
                api_key: input.api_key.map(str::to_owned),
                model: input.model.to_owned(),
            });
        if self.failure == StageFailure::Url {
            return Err(ConfigFailure {
                message: String::from("the fixture config has no URL"),
            });
        }
        Ok(self.complete_url(
            input.api_base,
            input.model,
            input.optional_params,
            input.litellm_params,
        ))
    }
}

// A config that leaves TranscriptionHttpConfig::complete_transcription_url at its default
pub(crate) struct DefaultUrlConfig(pub(crate) RecordingConfig);

impl AudioTranscriptionConfig for DefaultUrlConfig {
    fn supported_openai_params(&self, model: &str) -> Vec<String> {
        self.0.supported_openai_params(model)
    }

    fn transform_audio_request(
        &self,
        model: &str,
        audio_file: &AudioFile<'_>,
        optional_params: &Object,
        litellm_params: &Object,
    ) -> Result<AudioTranscriptionRequestData, TranscriptionContractError> {
        self.0
            .transform_audio_request(model, audio_file, optional_params, litellm_params)
    }

    fn supports_subtitle_synthesis(&self) -> bool {
        self.0.supports_subtitle_synthesis()
    }

    fn complete_url(
        &self,
        api_base: Option<&str>,
        model: &str,
        optional_params: &Object,
        litellm_params: &Object,
    ) -> String {
        self.0
            .complete_url(api_base, model, optional_params, litellm_params)
    }

    fn transform_audio_response(
        &self,
        raw_response: &UpstreamResponse,
    ) -> Result<TranscriptionResponse, TranscriptionContractError> {
        self.0.transform_audio_response(raw_response)
    }
}

impl TranscriptionHttpConfig for DefaultUrlConfig {
    fn validate_environment(&self, input: &Validation<'_>) -> Result<Headers, ConfigFailure> {
        self.0.validate_environment(input)
    }
}

pub(crate) struct ProxiedSource {
    pub(crate) client: ProxiedClient,
}

impl ClientSource for ProxiedSource {
    type Client = ProxiedClient;

    fn client(&self, _request: ClientRequest<'_>) -> Result<Self::Client, ClientError> {
        Ok(self.client.clone())
    }
}

pub(crate) fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("the object fixture is valid")
}

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the JSON fixture is valid")
}

pub(crate) fn record(data: AudioTranscriptionData) -> AudioTranscriptionRequestData {
    AudioTranscriptionRequestData {
        data,
        files: None,
        content_type: None,
    }
}

// The words a response carries, as the JSON both a hidden header and a subtitle case read
pub(crate) fn extra_words(response: &TranscriptionResponse) -> Option<Json> {
    response
        .extra
        .get("words")
        .map(|value| serde_json::from_value(value.clone()).expect("the words value is JSON"))
}

pub(crate) fn call<'call>(
    optional_params: &'call Object,
    litellm_params: &'call Object,
) -> TranscriptionCall<'call> {
    TranscriptionCall {
        model: "m",
        audio_file: AudioFile {
            filename: "a.wav",
            content: b"abc",
            content_type: "audio/wav",
        },
        optional_params,
        litellm_params,
        api_key: Some("sk-provider"),
        api_base: Some("https://fixture.test/v1"),
        custom_llm_provider: "openai",
        headers: None,
        atranscription: Json::Bool(false),
        timeout: None,
        max_retries: 0,
        shared_session: None,
    }
}

pub(crate) fn deps<'deps>(
    config: &'deps RecordingConfig,
    source: &'deps Source,
) -> TranscriptionDeps<'deps, RecordingConfig, Source> {
    TranscriptionDeps {
        config: Some(config),
        source,
        supplied: None,
    }
}

pub(crate) fn supplied<'deps>(
    config: &'deps RecordingConfig,
    source: &'deps Source,
    transport: &'deps Wire,
    kind: ClientKind,
) -> TranscriptionDeps<'deps, RecordingConfig, Source> {
    TranscriptionDeps {
        config: Some(config),
        source,
        supplied: Some(SuppliedClient { kind, transport }),
    }
}

pub(crate) fn body_bytes(body: &TranscriptionBody) -> Option<&[u8]> {
    match body {
        TranscriptionBody::Raw(data) => Some(data),
        TranscriptionBody::Encoded(RequestBody::Json(data)) => Some(data),
        TranscriptionBody::Encoded(RequestBody::Bytes { data, .. }) => Some(data),
        TranscriptionBody::Encoded(RequestBody::Empty | RequestBody::Multipart { .. }) => None,
    }
}

pub(crate) fn body_json(body: &TranscriptionBody) -> Json {
    let bytes = body_bytes(body).expect("the request carries bytes");
    serde_json::from_slice(bytes).expect("the request body is JSON")
}

pub(crate) fn header<'headers>(
    headers: &'headers HttpHeaders,
    name: &str,
) -> Option<&'headers str> {
    headers
        .get(name)
        .map(|value| value.to_str().expect("the header is text"))
}
