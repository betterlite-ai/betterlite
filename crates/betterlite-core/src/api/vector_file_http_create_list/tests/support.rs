use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::io;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialsSettings;
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
use betterlite_openai::api::vector_file_contracts::records::FileParams;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::vector_stores::Headers as OpenAiHeaders;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use http::header::HeaderMap as HttpHeaders;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::api::vector_file_http_create_list::config::{
    FilesConfig, FilesParams, OpenAiFilesConfig,
};
use crate::api::vector_file_http_create_list::error::FileTransportError;
use crate::api::vector_file_http_create_list::service::{
    CreateCall, FileCall, FileDeps, FileRequest, FileTarget, FileTransport, ListCall, LogContext,
    Operation, PreparedFile, prepare,
};
use crate::api::vector_store_http_write::error::{
    BaseLlmException, ClientError, HeaderMap, UrlFailure, header_map,
};
use crate::api::vector_store_http_write::service::{ClientKind, ClientSource};

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
            r#"{{"model":"vector-store-file","call_type":"{call_type}","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}}"#
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

    pub(crate) fn raw_only(&mut self) {
        self.state.request_logging.raw_request_only = true;
    }

    pub(crate) fn prepare<'deps, C, S>(
        &mut self,
        deps: &'deps FileDeps<'deps, C, S>,
        mode: ClientKind,
        call: &FileCall<'_>,
        events: &Events,
    ) -> Result<PreparedFile<'deps, C, S>, FileTransportError>
    where
        C: FilesConfig,
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
            mode,
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
    requests: Arc<Mutex<Vec<FileRequest>>>,
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

    pub(crate) fn requests(&self) -> MutexGuard<'_, Vec<FileRequest>> {
        self.requests.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl FileTransport for Wire {
    fn send(
        &self,
        request: FileRequest,
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

// The provider config's answers are observable through this stand-in
pub(crate) struct RecordingConfig {
    base: Option<String>,
}

impl RecordingConfig {
    pub(crate) fn new(base: &str) -> Self {
        Self {
            base: Some(base.to_owned()),
        }
    }

    pub(crate) fn without_base() -> Self {
        Self { base: None }
    }
}

impl FilesConfig for RecordingConfig {
    fn validate_environment(
        &self,
        mut headers: OpenAiHeaders,
        _params: &FilesParams,
    ) -> OpenAiHeaders {
        headers.insert(
            "Authorization".to_owned(),
            SecretString::from("Bearer sk-provider"),
        );
        headers.insert(
            "Content-Type".to_owned(),
            SecretString::from("application/json"),
        );
        headers.insert(
            "OpenAI-Beta".to_owned(),
            SecretString::from("assistants=v2"),
        );
        headers
    }

    fn complete_url(
        &self,
        _params: &FilesParams,
        _vector_store_id: &str,
    ) -> Result<String, UrlFailure> {
        let Some(base) = &self.base else {
            return Err(UrlFailure {
                source: Box::new(io::Error::other("the base is missing")),
            });
        };
        Ok(base.clone())
    }

    fn transform_response(&self, reply: &UpstreamResponse) -> Result<Json, BaseLlmException> {
        serde_json::from_slice(&reply.body).map_err(|source| {
            self.error_class(
                &source.to_string(),
                reply.status,
                header_map(&reply.headers),
            )
        })
    }

    fn error_class(&self, message: &str, status_code: u16, headers: HeaderMap) -> BaseLlmException {
        BaseLlmException {
            status_code,
            message: message.to_owned(),
            headers,
            status_code_is_synthesized: false,
        }
    }
}

pub(crate) fn openai_settings() -> OpenAiCredentialsSettings {
    OpenAiCredentialsSettings::default()
}

pub(crate) fn openai_config<'settings>(
    settings: &'settings OpenAiCredentialsSettings,
    env_api_key: &str,
) -> OpenAiFilesConfig<'settings> {
    OpenAiFilesConfig::new(
        settings,
        LiteLlmGlobals::default(),
        Some(SecretString::from(env_api_key)),
        None,
    )
}

pub(crate) fn params(api_base: &str) -> FilesParams {
    FilesParams {
        api_base: Some(api_base.to_owned()),
        api_key: None,
        ssl_verify: None,
    }
}

pub(crate) fn file_params(json: &str) -> FileParams {
    serde_json::from_str(json).expect("the file params fixture is valid")
}

pub(crate) fn extra(json: &str) -> IndexMap<String, Json> {
    serde_json::from_str(json).expect("the extra fixture is valid")
}

pub(crate) fn caller_headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
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

pub(crate) fn body_bytes(request: &FileRequest) -> Option<&[u8]> {
    match &request.body {
        RequestBody::Json(body) => Some(body),
        RequestBody::Bytes { data, .. } => Some(data),
        RequestBody::Empty | RequestBody::Multipart { .. } => None,
    }
}

pub(crate) fn body_json(request: &FileRequest) -> Json {
    let body = body_bytes(request).expect("the request carries a JSON body");
    serde_json::from_slice(body).expect("the request body is JSON")
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
        Self {
            client: proxied_client(limits),
        }
    }
}

impl ClientSource for ProxiedSource {
    type Client = ProxiedClient;

    fn client(&self, _ssl_verify: Option<bool>) -> Result<ProxiedClient, ClientError> {
        Ok(self.client.clone())
    }
}

// One call's inputs; a case sets only the overrides it exercises
pub(crate) struct CallRequest<'call> {
    pub(crate) vector_store_id: &'call str,
    pub(crate) params: &'call FilesParams,
    pub(crate) headers: Option<&'call BTreeMap<String, String>>,
    pub(crate) timeout: Option<Duration>,
}

impl<'call> CallRequest<'call> {
    pub(crate) const fn new(vector_store_id: &'call str, params: &'call FilesParams) -> Self {
        Self {
            vector_store_id,
            params,
            headers: None,
            timeout: None,
        }
    }

    pub(crate) const fn target(&self) -> FileTarget<'call> {
        FileTarget {
            vector_store_id: self.vector_store_id,
            params: self.params,
            extra_headers: self.headers,
            timeout: self.timeout,
        }
    }
}

pub(crate) fn request_with_headers<'call>(
    vector_store_id: &'call str,
    params: &'call FilesParams,
    headers: &'call BTreeMap<String, String>,
) -> CallRequest<'call> {
    CallRequest {
        headers: Some(headers),
        ..CallRequest::new(vector_store_id, params)
    }
}

pub(crate) fn file_deps<'deps, C, S>(config: &'deps C, source: &'deps S) -> FileDeps<'deps, C, S>
where
    C: FilesConfig,
    S: ClientSource,
{
    FileDeps {
        config,
        source,
        supplied: None,
    }
}

pub(crate) const fn file_call<'call>(
    request: &CallRequest<'call>,
    operation: Operation,
    fields: &'call FileParams,
    extra: Option<&'call IndexMap<String, Json>>,
) -> FileCall<'call> {
    let target = request.target();
    match operation {
        Operation::Create => FileCall::Create(CreateCall {
            target,
            create_request: fields,
            extra_body: extra,
        }),
        Operation::List => FileCall::List(ListCall {
            target,
            query_params: fields,
            extra_query: extra,
        }),
    }
}

pub(crate) const fn call_type(operation: Operation) -> &'static str {
    match operation {
        Operation::Create => "vector_store_file_create",
        Operation::List => "vector_store_file_list",
    }
}

pub(crate) fn request_header<'request>(
    request: &'request FileRequest,
    name: &str,
) -> Option<&'request str> {
    request.headers.get(name)?.to_str().ok()
}
