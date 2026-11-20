use std::future::{Future, ready};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialsSettings;
use betterlite_error::error::ErrorClass;
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
use betterlite_models::decisions::Json;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use http::HeaderMap as HttpHeaders;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::api::responses_lifecycle_write::call::{
    LifecycleOperation, LifecycleWriteParams, ResponsesLifecycleWriteCall,
};
use crate::api::responses_lifecycle_write::error::ResponsesLifecycleWriteError;
use crate::api::responses_write_http::config::OpenAiResponsesConfig;
use crate::api::responses_write_http::service::{
    ClientSource, SharedSession, WriteRequest, WriteTransport,
};
use crate::api::vector_store_http_write::error::ClientError;

pub(crate) const DELETED: &str = r#"{"id":"resp_a","object":"response","deleted":true}"#;
pub(crate) const CANCELLED: &str = r#"{"id":"resp_a","created_at":1735689600,"object":"response","status":"cancelled","model":"gpt-5","output":[]}"#;

// The cancelled reply names the same raw id a full response would, which is what the SDK returns unchanged
#[derive(Clone)]
pub(crate) struct Wire {
    status: u16,
    headers: HttpHeaders,
    body: Vec<u8>,
    failure: Arc<Mutex<Option<TransportError>>>,
    requests: Arc<Mutex<Vec<WriteRequest>>>,
}

impl Wire {
    pub(crate) fn returning(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: HttpHeaders::new(),
            body: body.as_bytes().to_vec(),
            failure: Arc::default(),
            requests: Arc::default(),
        }
    }

    pub(crate) fn requests(&self) -> MutexGuard<'_, Vec<WriteRequest>> {
        self.requests.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl WriteTransport for Wire {
    fn send(
        &self,
        request: WriteRequest,
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
            status: self.status,
            headers: self.headers.clone(),
            body: self.body.clone(),
        }))
    }
}

pub(crate) struct Source {
    wire: Wire,
}

impl Source {
    pub(crate) fn new(wire: Wire) -> Self {
        Self { wire }
    }
}

impl ClientSource for Source {
    type Client = Wire;

    fn client(
        &self,
        _ssl_verify: Option<bool>,
        _shared_session: Option<SharedSession>,
    ) -> Result<Wire, ClientError> {
        Ok(self.wire.clone())
    }
}

pub(crate) fn credentials() -> OpenAiCredentialsSettings {
    OpenAiCredentialsSettings::default()
}

// The provider config whose environment validation and endpoint both run, as B2417's own tests do
pub(crate) fn openai_config(settings: &OpenAiCredentialsSettings) -> OpenAiResponsesConfig<'_> {
    OpenAiResponsesConfig::new(
        settings,
        LiteLlmGlobals::default(),
        Some(SecretString::from("sk-provider")),
        None,
    )
}

// A call with only its operation and id set; each test adds the overrides it exercises
pub(crate) fn call(
    operation: LifecycleOperation,
    response_id: &str,
) -> ResponsesLifecycleWriteCall {
    ResponsesLifecycleWriteCall {
        operation,
        response_id: response_id.to_owned(),
        custom_llm_provider: None,
        extra_headers: None,
        extra_query: None,
        extra_body: None,
        timeout: None,
        params: LifecycleWriteParams::default(),
        adelete_responses: None,
        acancel_responses: None,
    }
}

pub(crate) fn extra(json: &str) -> IndexMap<String, Json> {
    serde_json::from_str(json).expect("the extras fixture is valid")
}

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the expected JSON parses")
}

pub(crate) fn body_bytes(request: &WriteRequest) -> Option<&[u8]> {
    match &request.body {
        RequestBody::Json(body) => Some(body),
        RequestBody::Bytes { data, .. } => Some(data),
        RequestBody::Empty | RequestBody::Multipart { .. } => None,
    }
}

pub(crate) fn body_json(request: &WriteRequest) -> Json {
    let body = body_bytes(request).expect("the request carries a JSON body");
    serde_json::from_slice(body).expect("the request body is JSON")
}

pub(crate) fn header(request: &WriteRequest, name: &str) -> Option<String> {
    request
        .headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

pub(crate) struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

const CLOCK: &str = "2026-01-01T00:00:05+00:00";

fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds")
}

fn clock() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(CLOCK).expect("the clock parses")
}

// The pre/post environment the handlers log into: no callbacks and no sink, so only the state is observable
pub(crate) struct Harness {
    redactor: Redactor,
    sink: Sink,
    now: DateTime<FixedOffset>,
}

impl Harness {
    pub(crate) fn new() -> Self {
        Self {
            redactor: redactor(),
            sink: Sink,
            now: clock(),
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

    pub(crate) fn class(&self, error: &ResponsesLifecycleWriteError) -> ErrorClass {
        error.error_class(&self.redactor)
    }
}

pub(crate) fn state(model: &str, call_id: &str) -> LoggingState {
    let init: LoggingInit = serde_json::from_str(&format!(
        r#"{{"model":"{model}","call_type":"responses","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"{call_id}","function_id":"fn-1","litellm_trace_id":"trace-1"}}"#
    ))
    .expect("the logging fixture is valid");
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor(),
        || String::from("generated"),
        Duration::default,
    )
    .expect("the logging state builds")
}
