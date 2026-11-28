use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::mem::take;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::vector_store_http_write::error::ClientError;
use betterlite_core::api::vector_store_http_write::service::ClientSource;
use betterlite_http::proxy::base_builder;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_openai::api::speech_provider_contract::TextToSpeechRequestData;
use betterlite_openai::files::records::BinaryContent;
use betterlite_openai::openai_base_config::Object;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use http::header::HeaderMap as HttpHeaders;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use crate::api::speech_http::error::SpeechError;
use crate::api::speech_http::{
    LogContext, Mode, SpeechBinaryResponse, SpeechCall, SpeechDeps, SpeechParams, SpeechTransport,
    SpeechWireRequest,
};
use crate::api::speech_provider_contract::error::{BaseLlmException, SpeechContractError};
use crate::api::speech_provider_contract::{
    ClassAttribute, SpeechHeaders, SpeechMappingOptions, SpeechRequestInput, TextToSpeechConfig,
};

pub(crate) const URL: &str = "http://provider.test/v1/audio/speech";

pub(crate) struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the expected JSON parses")
}

pub(crate) fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("the object fixture parses")
}

pub(crate) fn data(
    dict: Option<&str>,
    ssml: Option<&str>,
    provider: Option<SpeechHeaders>,
) -> TextToSpeechRequestData {
    TextToSpeechRequestData {
        dict_body: dict.map(object),
        ssml_body: ssml.map(ToOwned::to_owned),
        headers: provider,
    }
}

// The pre-call a production request runs, observable through the state it records
pub(crate) struct Fixture {
    redactor: Redactor,
    sink: Sink,
    state: LoggingState,
    now: DateTime<FixedOffset>,
}

impl Fixture {
    pub(crate) fn new() -> Self {
        let init: LoggingInit = serde_json::from_str(
            r#"{"model":"speech-model","call_type":"speech","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
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

pub(crate) async fn run<'deps, S>(
    fixture: &mut Fixture,
    deps: &'deps SpeechDeps<'deps, S>,
    mode: Mode,
    call: &SpeechCall<'_>,
) -> Result<SpeechBinaryResponse, SpeechError>
where
    S: ClientSource<Client: SpeechTransport>,
{
    let env = PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor: &fixture.redactor,
        log: &fixture.sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: fixture.now,
    };
    let mut log = LogContext {
        state: &mut fixture.state,
        env: &env,
    };
    match mode {
        Mode::Sync => deps.text_to_speech_handler(call, &mut log).await,
        Mode::Asynchronous => deps.async_text_to_speech_handler(call, &mut log).await,
    }
}

#[derive(Clone)]
pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) headers: HttpHeaders,
    pub(crate) body: Vec<u8>,
}

impl Reply {
    pub(crate) fn bytes(status: u16, body: &[u8]) -> Self {
        Self {
            status,
            headers: HttpHeaders::new(),
            body: body.to_vec(),
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
    requests: Arc<Mutex<Vec<SpeechWireRequest>>>,
    default_timeout: Duration,
}

impl Wire {
    pub(crate) fn replying(reply: Reply) -> Self {
        Self {
            reply,
            failure: Arc::default(),
            requests: Arc::default(),
            default_timeout: Duration::from_secs(60),
        }
    }

    pub(crate) fn failing(error: TransportError) -> Self {
        let wire = Self::replying(Reply::bytes(200, b""));
        *wire.failure.lock().unwrap_or_else(PoisonError::into_inner) = Some(error);
        wire
    }

    pub(crate) fn requests(&self) -> MutexGuard<'_, Vec<SpeechWireRequest>> {
        self.requests.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl SpeechTransport for Wire {
    fn default_timeout(&self) -> Duration {
        self.default_timeout
    }

    fn send(
        &self,
        request: SpeechWireRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
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

pub(crate) struct Source {
    wire: Wire,
    calls: Arc<Mutex<Vec<Option<bool>>>>,
}

impl Source {
    pub(crate) fn new(wire: Wire) -> Self {
        Self {
            wire,
            calls: Arc::default(),
        }
    }

    pub(crate) fn calls(&self) -> Vec<Option<bool>> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ClientSource for Source {
    type Client = Wire;

    fn client(&self, ssl_verify: Option<bool>) -> Result<Wire, ClientError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ssl_verify);
        Ok(self.wire.clone())
    }
}

// The standard speech call a test starts from: one wire, its source and the recording config over them
pub(crate) struct Harness {
    pub(crate) wire: Wire,
    pub(crate) source: Source,
    pub(crate) config: RecordingConfig,
    pub(crate) declared: SpeechParams,
    pub(crate) optional: Object,
    pub(crate) litellm: Object,
    pub(crate) caller: SpeechHeaders,
    pub(crate) voice: Option<&'static str>,
    pub(crate) timeout: Option<Duration>,
}

impl Harness {
    pub(crate) fn new(reply: Reply) -> Self {
        Self::from_wire(Wire::replying(reply))
    }

    pub(crate) fn from_wire(wire: Wire) -> Self {
        let source = Source::new(wire.clone());
        Self {
            wire,
            source,
            config: RecordingConfig::new(
                Ok(URL.to_owned()),
                data(Some(r#"{"input":"hello"}"#), None, None),
                SpeechHeaders::new(),
            ),
            declared: params(None, Some(URL), None),
            optional: Object::new(),
            litellm: Object::new(),
            caller: SpeechHeaders::new(),
            voice: None,
            timeout: None,
        }
    }

    pub(crate) fn deps(&self) -> SpeechDeps<'_, Source> {
        SpeechDeps {
            config: &self.config,
            source: &self.source,
            supplied: None,
        }
    }

    pub(crate) fn call(&self) -> SpeechCall<'_> {
        SpeechCall {
            model: "tts-1",
            input: "hello",
            voice: self.voice,
            optional_params: &self.optional,
            litellm_params: &self.litellm,
            extra_headers: (!self.caller.is_empty()).then_some(&self.caller),
            params: &self.declared,
            timeout: self.timeout,
        }
    }
}

pub(crate) const IMPATIENT: TransportLimits = TransportLimits {
    read_timeout: Duration::from_millis(100),
    ..TransportLimits::DEFAULT
};

pub(crate) struct ProxiedSource {
    client: ProxiedClient,
}

impl ProxiedSource {
    pub(crate) fn new(limits: TransportLimits) -> Self {
        let policy = Arc::new(
            ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
                .expect("the proxy policy resolves"),
        );
        Self {
            client: ProxiedClient::build(base_builder().http1_only(), &policy, limits)
                .expect("the proxied client builds"),
        }
    }
}

impl ClientSource for ProxiedSource {
    type Client = ProxiedClient;

    fn client(&self, _ssl_verify: Option<bool>) -> Result<ProxiedClient, ClientError> {
        Ok(self.client.clone())
    }
}

pub(crate) struct Validation {
    pub(crate) api_key: Option<String>,
    pub(crate) api_base: Option<String>,
    pub(crate) headers: SpeechHeaders,
}

pub(crate) struct SpeechObserved {
    pub(crate) model: String,
    pub(crate) input: String,
    pub(crate) voice: Option<String>,
    pub(crate) optional_params: Object,
    pub(crate) litellm_params: Object,
    pub(crate) headers: SpeechHeaders,
}

// The validated and transformed answers the case chooses, with every call into it on record
pub(crate) struct RecordingConfig {
    pub(crate) url: Result<String, SpeechContractError>,
    pub(crate) data: TextToSpeechRequestData,
    pub(crate) validated: SpeechHeaders,
    validations: Mutex<Vec<Validation>>,
    observed: Mutex<Vec<SpeechObserved>>,
    errors: Mutex<Vec<(String, u16, SpeechHeaders)>>,
    responses: Mutex<usize>,
}

impl RecordingConfig {
    pub(crate) fn new(
        url: Result<String, SpeechContractError>,
        data: TextToSpeechRequestData,
        validated: SpeechHeaders,
    ) -> Self {
        Self {
            url,
            data,
            validated,
            validations: Mutex::default(),
            observed: Mutex::default(),
            errors: Mutex::default(),
            responses: Mutex::default(),
        }
    }

    pub(crate) fn validations(&self) -> Vec<Validation> {
        take(
            &mut *self
                .validations
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        )
    }

    pub(crate) fn observed(&self) -> Vec<SpeechObserved> {
        take(&mut *self.observed.lock().unwrap_or_else(PoisonError::into_inner))
    }

    pub(crate) fn errors(&self) -> Vec<(String, u16, SpeechHeaders)> {
        self.errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn responses(&self) -> usize {
        *self
            .responses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl TextToSpeechConfig for RecordingConfig {
    fn class_attributes(&self) -> IndexMap<String, ClassAttribute> {
        IndexMap::new()
    }

    fn supported_openai_params(&self, _model: &str) -> Vec<String> {
        Vec::new()
    }

    fn map_openai_params(
        &self,
        _model: &str,
        _optional_params: Object,
        _options: &SpeechMappingOptions,
    ) -> (Option<String>, Object) {
        (None, Object::new())
    }

    fn validate_environment(
        &self,
        headers: &SpeechHeaders,
        _model: &str,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> SpeechHeaders {
        self.validations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Validation {
                api_key: api_key.map(|key| key.expose_secret().to_owned()),
                api_base: api_base.map(ToOwned::to_owned),
                headers: headers.clone(),
            });
        self.validated.clone()
    }

    fn complete_url(
        &self,
        _model: &str,
        _api_base: Option<&str>,
        _litellm_params: &Object,
    ) -> Result<String, SpeechContractError> {
        self.url.clone()
    }

    fn speech_request(&self, request: &SpeechRequestInput<'_>) -> TextToSpeechRequestData {
        self.observed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(SpeechObserved {
                model: request.model.to_owned(),
                input: request.input.to_owned(),
                voice: request.voice.map(ToOwned::to_owned),
                optional_params: request.optional_params.clone(),
                litellm_params: request.litellm_params.clone(),
                headers: request.headers.clone(),
            });
        self.data.clone()
    }

    fn speech_response(&self, _model: &str, raw_response: UpstreamResponse) -> BinaryContent {
        let mut responses = self
            .responses
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *responses = responses.saturating_add(1);
        drop(responses);
        BinaryContent::from_response(raw_response)
    }

    fn get_error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: &SpeechHeaders,
    ) -> BaseLlmException {
        self.errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((error_message.to_owned(), status_code, headers.clone()));
        BaseLlmException {
            status_code,
            message: error_message.to_owned(),
            headers: headers.clone(),
        }
    }
}

pub(crate) fn headers(pairs: &[(&str, &str)]) -> SpeechHeaders {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn params(
    api_key: Option<&str>,
    api_base: Option<&str>,
    ssl_verify: Option<bool>,
) -> SpeechParams {
    SpeechParams {
        api_key: api_key.map(SecretString::from),
        api_base: api_base.map(ToOwned::to_owned),
        ssl_verify,
    }
}

pub(crate) fn body_bytes(request: &SpeechWireRequest) -> Option<&[u8]> {
    match &request.body {
        RequestBody::Json(body) => Some(body),
        RequestBody::Bytes { data, .. } => Some(data),
        RequestBody::Empty | RequestBody::Multipart { .. } => None,
    }
}

pub(crate) fn body_json(request: &SpeechWireRequest) -> Json {
    let body = body_bytes(request).expect("the request carries a body");
    serde_json::from_slice(body).expect("the request body is JSON")
}

pub(crate) fn bytes_content_type(request: &SpeechWireRequest) -> Option<&str> {
    match &request.body {
        RequestBody::Bytes { content_type, .. } => content_type.to_str().ok(),
        RequestBody::Empty | RequestBody::Json(_) | RequestBody::Multipart { .. } => None,
    }
}

pub(crate) fn header(request: &SpeechWireRequest, name: &str) -> Option<String> {
    request
        .headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}

// One wire request as data, so two runs can be compared without holding either client's lock
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Capture {
    pub(crate) method: String,
    pub(crate) url: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Option<Vec<u8>>,
}

pub(crate) fn captured(wire: &Wire) -> Vec<Capture> {
    wire.requests().iter().map(capture_one).collect()
}

fn capture_one(request: &SpeechWireRequest) -> Capture {
    Capture {
        method: request.method.as_str().to_owned(),
        url: request.url.to_string(),
        headers: capture_headers(request),
        body: body_bytes(request).map(<[u8]>::to_vec),
    }
}

fn capture_headers(request: &SpeechWireRequest) -> Vec<(String, String)> {
    let mut headers: Vec<(String, String)> = request
        .headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|text| (name.as_str().to_owned(), text.to_owned()))
        })
        .collect();
    headers.sort();
    headers
}
