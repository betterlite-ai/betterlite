use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::io::Error;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialsSettings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, LoggerFn, PrePostEnv,
};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::vector_stores::Headers as OpenAiHeaders;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use http::HeaderMap as HttpHeaders;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::api::vector_file_http_read::VectorFileReadTransport;
use crate::api::vector_file_http_read::config::{
    OpenAiReadConfig, QueryParams, ReadConfig, ReadParams,
};
use crate::api::vector_file_http_read::error::{PrepareFailure, ReadError};
use crate::api::vector_file_http_read::service::{
    LogContext, Operation, ReadCall, ReadRequest, ReadTarget, ReadTransport,
};
use crate::api::vector_store_http_write::error::{BaseLlmException, ClientError, HeaderMap};
use crate::api::vector_store_http_write::service::{ClientKind, ClientSource, SuppliedClient};

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

struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

// Everything a PrePostEnv borrows besides the logging state, so a call can borrow both halves at once
struct Parts {
    redactor: Redactor,
    sink: Sink,
    now: DateTime<FixedOffset>,
}

impl Parts {
    fn env<'env>(&'env self, logger: Option<LoggerFn<'env>>) -> PrePostEnv<'env> {
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
            logger_fn: logger,
            now: self.now,
        }
    }
}

pub(crate) struct Fixture {
    parts: Parts,
    pub(crate) state: LoggingState,
    events: Events,
}

impl Fixture {
    pub(crate) fn new(call_type: &str, events: &Events) -> Self {
        let init: LoggingInit = serde_json::from_str(&format!(
            r#"{{"model":"vector-store","call_type":"{call_type}","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}}"#
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
            parts: Parts {
                redactor,
                sink: Sink,
                now,
            },
            state,
            events: events.clone(),
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
    requests: Arc<Mutex<Vec<ReadRequest>>>,
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

    pub(crate) fn requests(&self) -> Vec<ReadRequest> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ReadTransport for Wire {
    fn send(
        &self,
        request: ReadRequest,
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

pub(crate) fn marker(text: &str) -> Json {
    Json::Object(IndexMap::from([(
        String::from("selected"),
        Json::Text(String::from(text)),
    )]))
}

// The error converter's answers are observable through this config, and its transforms are marked so a swapped one shows
pub(crate) struct RecordingConfig {
    base: String,
    query: QueryParams,
    refuse_url: bool,
    errors: Mutex<Vec<(String, u16, HeaderMap)>>,
}

impl RecordingConfig {
    pub(crate) fn new(base: &str) -> Self {
        Self {
            base: base.to_owned(),
            query: QueryParams::new(),
            refuse_url: false,
            errors: Mutex::default(),
        }
    }

    pub(crate) fn with_query(mut self, query: QueryParams) -> Self {
        self.query = query;
        self
    }

    pub(crate) fn refusing_url(mut self) -> Self {
        self.refuse_url = true;
        self
    }

    pub(crate) fn errors(&self) -> Vec<(String, u16, HeaderMap)> {
        self.errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ReadConfig for RecordingConfig {
    fn validate_environment(
        &self,
        mut headers: OpenAiHeaders,
        _params: &ReadParams,
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
        _params: &ReadParams,
        _vector_store_id: &str,
    ) -> Result<String, PrepareFailure> {
        if self.refuse_url {
            return Err(PrepareFailure {
                source: Box::new(Error::other("the base URL was refused")),
            });
        }
        Ok(self.base.clone())
    }

    fn transform_retrieve_request(
        &self,
        file_id: &str,
        api_base: &str,
    ) -> Result<(String, QueryParams), PrepareFailure> {
        Ok((format!("{api_base}/files/{file_id}"), self.query.clone()))
    }

    fn transform_content_request(
        &self,
        file_id: &str,
        api_base: &str,
    ) -> Result<(String, QueryParams), PrepareFailure> {
        Ok((
            format!("{api_base}/files/{file_id}/content"),
            self.query.clone(),
        ))
    }

    fn transform_retrieve_response(
        &self,
        _response: &http::Response<Vec<u8>>,
    ) -> Result<Json, BaseLlmException> {
        Ok(marker("retrieve"))
    }

    fn transform_content_response(
        &self,
        _response: &http::Response<Vec<u8>>,
    ) -> Result<Json, BaseLlmException> {
        Ok(marker("content"))
    }

    fn error_class(&self, message: &str, status_code: u16, headers: HeaderMap) -> BaseLlmException {
        self.errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((message.to_owned(), status_code, headers.clone()));
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
) -> OpenAiReadConfig<'settings> {
    OpenAiReadConfig::new(
        settings,
        LiteLlmGlobals::default(),
        Some(SecretString::from(env_api_key)),
        None,
    )
}

pub(crate) fn params(api_base: &str) -> ReadParams {
    ReadParams {
        api_base: Some(api_base.to_owned()),
        api_key: None,
        ssl_verify: None,
    }
}

pub(crate) fn caller_headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

// One handler call's inputs: the two ids, the params, the overrides and the client the caller supplies
pub(crate) struct Invocation<'call> {
    pub(crate) vector_store_id: &'call str,
    pub(crate) file_id: &'call str,
    pub(crate) params: &'call ReadParams,
    pub(crate) headers: Option<&'call BTreeMap<String, String>>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) operation: Operation,
    pub(crate) mode: ClientKind,
    pub(crate) supplied: Option<SuppliedClient<'call, Wire>>,
}

pub(crate) const fn invocation<'call>(
    vector_store_id: &'call str,
    file_id: &'call str,
    params: &'call ReadParams,
    operation: Operation,
    mode: ClientKind,
) -> Invocation<'call> {
    Invocation {
        vector_store_id,
        file_id,
        params,
        headers: None,
        timeout: None,
        operation,
        mode,
        supplied: None,
    }
}

impl<'call> Invocation<'call> {
    fn target(&self) -> ReadTarget<'call> {
        ReadTarget {
            vector_store_id: self.vector_store_id,
            file_id: self.file_id,
            params: self.params,
            extra_headers: self.headers,
            timeout: self.timeout,
        }
    }
}

pub(crate) async fn invoke<C>(
    transport: &VectorFileReadTransport<Source>,
    config: &C,
    fixture: &mut Fixture,
    invocation: Invocation<'_>,
) -> Result<Json, ReadError>
where
    C: ReadConfig,
{
    let call = ReadCall {
        target: invocation.target(),
        client: invocation.supplied,
        mode: invocation.mode,
        operation: invocation.operation,
    };
    let events = fixture.events.clone();
    let logger = move |_details: &Json| -> Result<(), PrePostError> {
        events.push("log");
        Ok(())
    };
    let env = fixture.parts.env(Some(&logger));
    let log = LogContext {
        state: &mut fixture.state,
        env: &env,
    };
    transport.read(config, call, log).await
}
